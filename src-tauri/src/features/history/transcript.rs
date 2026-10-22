//! A conversation as the chat pane shows it, built from the same events the
//! UI gets (`ui/src/lib/chat.ts` does the same in TypeScript). Pure, so the
//! recording logic is tested without a process.

use serde::{Deserialize, Serialize};

use crate::ai::AiEvent;

/// Characters of questions and answers kept for the search filter.
const SEARCH_CHARS: usize = 8_000;
/// A title longer than this is cut.
const TITLE_CHARS: usize = 120;

/// One line of a saved conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatEntry {
    /// Your message, without the desktop context block. `context`: what
    /// the AI was told about the screen ("Downloads · 2 selected").
    User {
        text: String,
        context: Option<String>,
    },
    /// Answer text (Markdown).
    Assistant {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        summary: String,
        state: ToolState,
    },
    Approval {
        id: String,
        tool: String,
        summary: String,
        detail: String,
        state: ApprovalState,
    },
    Error {
        text: String,
    },
    /// A quiet status line ("Stopped").
    Note {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Running,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Allowed,
    Denied,
}

/// A conversation being recorded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Transcript {
    pub entries: Vec<ChatEntry>,
    /// What `Started` said last ("claude-sonnet-5-5").
    pub model: String,
    /// A turn is in progress (between your message and its end).
    running: bool,
    /// Where a reply began that arrived before its question was recorded
    /// (the question is recorded once the ask returns, and a warm process
    /// may have spoken already): the question goes there.
    reply_start: Option<usize>,
    /// That early reply has ended too (a quick error), so the question's
    /// turn is over as soon as it's recorded.
    ended_early: bool,
}

impl Transcript {
    /// A saved conversation, to carry on recording.
    pub fn from_saved(entries: Vec<ChatEntry>, model: String) -> Self {
        Self {
            entries: settled(entries),
            model,
            ..Self::default()
        }
    }

    /// Records your message; its turn starts (unless it already ended).
    pub fn user_message(&mut self, text: &str, context: Option<&str>) {
        let entry = ChatEntry::User {
            text: text.to_owned(),
            context: context.map(str::to_owned),
        };
        match self.reply_start.take() {
            Some(at) if at <= self.entries.len() => self.entries.insert(at, entry),
            _ => self.entries.push(entry),
        }
        self.running = !std::mem::take(&mut self.ended_early);
    }

    /// A turn is in progress.
    #[cfg(test)]
    pub fn running(&self) -> bool {
        self.running
    }

    /// Applies one event; true when it ends a turn (time to save).
    pub fn apply(&mut self, event: &AiEvent) -> bool {
        match event {
            AiEvent::Started { model } => self.model.clone_from(model),
            // Between turns, text starts a new answer (the last one ended).
            AiEvent::TextDelta { text } => match self.entries.last_mut() {
                Some(ChatEntry::Assistant { text: answer })
                    if self.running || self.reply_start.is_some() =>
                {
                    answer.push_str(text)
                }
                _ => self.push(ChatEntry::Assistant { text: text.clone() }),
            },
            AiEvent::ToolStarted { id, name, summary } => self.push(ChatEntry::Tool {
                id: id.clone(),
                name: name.clone(),
                summary: summary.clone(),
                state: ToolState::Running,
            }),
            AiEvent::ToolFinished { id, ok } => {
                for entry in &mut self.entries {
                    if let ChatEntry::Tool {
                        id: tool, state, ..
                    } = entry
                    {
                        if tool == id {
                            *state = if *ok {
                                ToolState::Ok
                            } else {
                                ToolState::Failed
                            };
                        }
                    }
                }
            }
            AiEvent::ApprovalRequested {
                id,
                tool,
                summary,
                detail,
            } => self.push(ChatEntry::Approval {
                id: id.clone(),
                tool: tool.clone(),
                summary: summary.clone(),
                detail: detail.clone(),
                state: ApprovalState::Pending,
            }),
            AiEvent::ApprovalResolved { id, allowed } => {
                for entry in &mut self.entries {
                    if let ChatEntry::Approval {
                        id: approval,
                        state,
                        ..
                    } = entry
                    {
                        if approval == id {
                            *state = if *allowed {
                                ApprovalState::Allowed
                            } else {
                                ApprovalState::Denied
                            };
                        }
                    }
                }
            }
            AiEvent::Done => return self.end_turn(),
            AiEvent::Stopped => {
                self.push(ChatEntry::Note {
                    text: "Stopped".into(),
                });
                return self.end_turn();
            }
            AiEvent::Error { message } => {
                self.push(ChatEntry::Error {
                    text: message.clone(),
                });
                return self.end_turn();
            }
        }
        false
    }

    fn push(&mut self, entry: ChatEntry) {
        if !self.running && self.reply_start.is_none() {
            self.reply_start = Some(self.entries.len());
        }
        self.entries.push(entry);
    }

    fn end_turn(&mut self) -> bool {
        if !self.running && self.reply_start.is_some() {
            self.ended_early = true;
        }
        self.running = false;
        true
    }
}

/// A saved conversation can't continue a turn: tools still running were
/// cut off, and approvals nobody answered can't be answered now.
pub fn settled(mut entries: Vec<ChatEntry>) -> Vec<ChatEntry> {
    for entry in &mut entries {
        match entry {
            ChatEntry::Tool { state, .. } if *state == ToolState::Running => {
                *state = ToolState::Failed
            }
            ChatEntry::Approval { state, .. } if *state == ApprovalState::Pending => {
                *state = ApprovalState::Denied
            }
            _ => {}
        }
    }
    entries
}

/// Questions plus answered turns: what a person would count as messages.
pub fn message_count(entries: &[ChatEntry]) -> u32 {
    let mut count = 0;
    let mut answered = false;
    for entry in entries {
        match entry {
            ChatEntry::User { .. } => {
                count += 1;
                answered = false;
            }
            ChatEntry::Assistant { .. } if !answered => {
                count += 1;
                answered = true;
            }
            _ => {}
        }
    }
    count
}

/// Lowercase questions and answers, cut to a size that's quick to search.
pub fn search_text(entries: &[ChatEntry]) -> String {
    let mut text = String::new();
    for entry in entries {
        if let ChatEntry::User { text: t, .. } | ChatEntry::Assistant { text: t } = entry {
            text.push_str(&t.to_lowercase());
            text.push('\n');
        }
        if text.len() >= SEARCH_CHARS {
            break;
        }
    }
    cut(&text, SEARCH_CHARS)
}

/// A conversation's title: its first question on one line, cut short.
pub fn title(question: &str) -> String {
    let line = question.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= TITLE_CHARS {
        return line;
    }
    let kept: String = line.chars().take(TITLE_CHARS - 1).collect();
    format!("{}…", kept.trim_end())
}

/// `text` cut to at most `max` bytes, on a character boundary.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Entries as saved in the database (a JSON array).
pub fn to_json(entries: &[ChatEntry]) -> String {
    serde_json::to_string(entries).unwrap_or_else(|_| "[]".into())
}

/// Entries from the database. A line this version doesn't know is skipped
/// rather than losing the whole conversation.
pub fn from_json(json: &str) -> Vec<ChatEntry> {
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    values
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(text: &str) -> AiEvent {
        AiEvent::TextDelta { text: text.into() }
    }

    fn user(text: &str) -> ChatEntry {
        ChatEntry::User {
            text: text.into(),
            context: None,
        }
    }

    fn assistant(text: &str) -> ChatEntry {
        ChatEntry::Assistant { text: text.into() }
    }

    #[test]
    fn builds_what_the_chat_pane_shows() {
        let mut t = Transcript::default();
        assert!(!t.apply(&AiEvent::Started {
            model: "claude-sonnet-5-5".into()
        }));
        t.user_message("sort my downloads", Some("Downloads · 2 selected"));
        t.apply(&delta("Sure, "));
        t.apply(&delta("moving them."));
        t.apply(&AiEvent::ToolStarted {
            id: "t1".into(),
            name: "Bash".into(),
            summary: "mv a b".into(),
        });
        t.apply(&AiEvent::ApprovalRequested {
            id: "r1".into(),
            tool: "Bash".into(),
            summary: "mv a b".into(),
            detail: "mv a b".into(),
        });
        t.apply(&AiEvent::ApprovalResolved {
            id: "r1".into(),
            allowed: true,
        });
        t.apply(&AiEvent::ToolFinished {
            id: "t1".into(),
            ok: true,
        });
        t.apply(&delta("Done."));
        assert!(t.apply(&AiEvent::Done));
        assert_eq!(t.model, "claude-sonnet-5-5");
        assert_eq!(
            t.entries,
            [
                ChatEntry::User {
                    text: "sort my downloads".into(),
                    context: Some("Downloads · 2 selected".into()),
                },
                assistant("Sure, moving them."),
                ChatEntry::Tool {
                    id: "t1".into(),
                    name: "Bash".into(),
                    summary: "mv a b".into(),
                    state: ToolState::Ok,
                },
                ChatEntry::Approval {
                    id: "r1".into(),
                    tool: "Bash".into(),
                    summary: "mv a b".into(),
                    detail: "mv a b".into(),
                    state: ApprovalState::Allowed,
                },
                assistant("Done."),
            ]
        );
        assert_eq!(message_count(&t.entries), 2);
    }

    #[test]
    fn stops_and_errors_end_a_turn_with_a_line() {
        let mut t = Transcript::default();
        t.user_message("long story", None);
        t.apply(&delta("Once"));
        assert!(t.apply(&AiEvent::Stopped));
        t.user_message("again", None);
        assert!(t.apply(&AiEvent::Error {
            message: "Claude Code exited".into()
        }));
        assert_eq!(
            t.entries,
            [
                user("long story"),
                assistant("Once"),
                ChatEntry::Note {
                    text: "Stopped".into()
                },
                user("again"),
                ChatEntry::Error {
                    text: "Claude Code exited".into()
                },
            ]
        );
    }

    #[test]
    fn a_reply_that_beats_its_question_goes_after_it() {
        let mut t = Transcript::default();
        t.user_message("hi", None);
        t.apply(&delta("Hello"));
        t.apply(&AiEvent::Done);
        // The follow-up's answer arrives before the follow-up is recorded.
        t.apply(&delta("Sure"));
        t.user_message("more", None);
        t.apply(&delta(" thing"));
        assert_eq!(
            t.entries,
            [
                user("hi"),
                assistant("Hello"),
                user("more"),
                assistant("Sure thing"),
            ]
        );

        assert!(t.running());

        // Even an error from start-up, before the first question: that
        // question's turn is over once it's recorded.
        let mut t = Transcript::default();
        t.apply(&AiEvent::Error {
            message: "signed out".into(),
        });
        t.user_message("hi", None);
        assert_eq!(t.entries[0], user("hi"));
        assert!(!t.running());
        t.user_message("again", None);
        assert!(t.running(), "the next question runs as usual");
    }

    #[test]
    fn saved_conversations_settle_what_was_cut_off() {
        let entries = vec![
            ChatEntry::Tool {
                id: "1".into(),
                name: "Read".into(),
                summary: String::new(),
                state: ToolState::Running,
            },
            ChatEntry::Approval {
                id: "2".into(),
                tool: "Bash".into(),
                summary: String::new(),
                detail: "rm x".into(),
                state: ApprovalState::Pending,
            },
        ];
        let t = Transcript::from_saved(entries, "gpt-6".into());
        assert!(matches!(
            t.entries[0],
            ChatEntry::Tool {
                state: ToolState::Failed,
                ..
            }
        ));
        assert!(matches!(
            t.entries[1],
            ChatEntry::Approval {
                state: ApprovalState::Denied,
                ..
            }
        ));
    }

    #[test]
    fn counts_questions_and_answered_turns() {
        let tool = ChatEntry::Tool {
            id: "1".into(),
            name: "Read".into(),
            summary: String::new(),
            state: ToolState::Ok,
        };
        let entries = [
            user("a"),
            assistant("checking"),
            tool,
            assistant("done"),
            user("b"),
            ChatEntry::Error { text: "x".into() },
        ];
        assert_eq!(message_count(&entries), 3);
        assert_eq!(message_count(&[]), 0);
    }

    #[test]
    fn search_text_is_lowercase_and_capped() {
        let entries = [user("Sort My Downloads"), assistant("Moved **3** files")];
        assert_eq!(
            search_text(&entries),
            "sort my downloads\nmoved **3** files\n"
        );
        let long = [assistant(&"é".repeat(SEARCH_CHARS))];
        let text = search_text(&long);
        assert!(text.len() <= SEARCH_CHARS && text.starts_with('é'));
    }

    #[test]
    fn titles_are_one_short_line() {
        assert_eq!(title("  what's\n the   weather? "), "what's the weather?");
        let long = title(&"word ".repeat(100));
        assert_eq!(long.chars().count(), TITLE_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn entries_round_trip_through_json() {
        let entries = vec![
            ChatEntry::User {
                text: "hi".into(),
                context: Some("Desktop".into()),
            },
            assistant("**hello**"),
            ChatEntry::Approval {
                id: "r1".into(),
                tool: "Write".into(),
                summary: "notes.txt".into(),
                detail: "text".into(),
                state: ApprovalState::Denied,
            },
            ChatEntry::Note {
                text: "Stopped".into(),
            },
        ];
        let json = to_json(&entries);
        assert!(json.contains(r#""kind":"user""#) && json.contains(r#""state":"denied""#));
        assert_eq!(from_json(&json), entries);
    }

    #[test]
    fn unknown_lines_are_skipped_not_fatal() {
        let json = r#"[{"kind":"user","text":"hi","context":null},{"kind":"hologram"},{"kind":"note","text":"Stopped"}]"#;
        assert_eq!(
            from_json(json),
            [
                user("hi"),
                ChatEntry::Note {
                    text: "Stopped".into()
                }
            ]
        );
        assert!(from_json("not json").is_empty());
    }
}
