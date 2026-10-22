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
mod tests;
