//! Expansion as you type, minus the Windows parts: which keywords expand,
//! the last few characters typed, and what each key press means for them.
//! The hook (`win.rs`) turns key presses into `Input`s and feeds them here.

use super::SnippetSettings;

/// Characters remembered; longer keywords can't expand.
const REMEMBERED: usize = 64;

/// Windows virtual-key codes this needs.
mod vk {
    pub const BACK: u16 = 0x08;
    /// Shift, Ctrl, Alt (either side), Caps/Num/Scroll Lock and the Windows
    /// keys: pressing one alone types nothing and changes nothing.
    pub const MODIFIERS: [u16; 14] = [
        0x10, 0x11, 0x12, 0x14, 0x5B, 0x5C, 0x90, 0x91, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
    ];
}

/// A snippet that expands as you type: its keyword and text (placeholders
/// not filled in yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub keyword: String,
    pub text: String,
}

/// The keywords that expand as you type: those starting with the trigger
/// (all of them when it's empty), and longer than it.
pub fn expansions(settings: &SnippetSettings) -> Vec<Expansion> {
    let trigger = settings.trigger.trim();
    settings
        .items
        .iter()
        .filter(|s| {
            let keyword = s.keyword.trim();
            keyword.starts_with(trigger) && keyword.chars().count() > trigger.chars().count()
        })
        .map(|s| Expansion {
            keyword: s.keyword.trim().to_owned(),
            text: s.text.clone(),
        })
        .collect()
}

/// What a key typed, per the keyboard layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    Text(String),
    /// An accent waiting for the next letter (é = ´ then e).
    Dead,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// A key went down in `window`, with these modifiers held.
    Key {
        vk: u16,
        typed: Typed,
        ctrl: bool,
        alt: bool,
        win: bool,
        window: isize,
    },
    /// A mouse button: the caret may have moved.
    Click,
}

/// A keyword was just typed: erase that many characters, then paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub erase: usize,
    pub expansion: Expansion,
}

/// Remembers the characters typed since the caret last jumped.
#[derive(Debug, Default)]
pub struct Typing {
    typed: Vec<char>,
    window: isize,
}

impl Typing {
    pub fn reset(&mut self) {
        self.typed.clear();
    }

    /// Takes one input; returns a keyword it completed, if any.
    pub fn feed(&mut self, input: Input, expansions: &[Expansion]) -> Option<Found> {
        let Input::Key {
            vk,
            typed,
            ctrl,
            alt,
            win,
            window,
        } = input
        else {
            self.reset();
            return None;
        };
        if window != self.window {
            self.reset();
            self.window = window;
        }
        if vk::MODIFIERS.contains(&vk) {
            return None;
        }
        // Ctrl or Alt alone make a shortcut (Ctrl+A, Alt+Tab) that can move
        // the caret; together they're AltGr, which types characters.
        if win || ctrl != alt {
            self.reset();
            return None;
        }
        if vk == vk::BACK {
            self.typed.pop();
            return None;
        }
        match typed {
            Typed::Dead => None,
            // Enter, Tab, Esc type control characters; arrows, Home, Delete
            // and the like type nothing. Either way the caret moved.
            Typed::Nothing => {
                self.reset();
                None
            }
            Typed::Text(text) if text.chars().any(char::is_control) => {
                self.reset();
                None
            }
            Typed::Text(text) => {
                self.typed.extend(text.chars());
                let excess = self.typed.len().saturating_sub(REMEMBERED);
                self.typed.drain(..excess);
                let found = self.completed(expansions)?;
                self.reset();
                Some(found)
            }
        }
    }

    /// The longest keyword the typed text ends with (letters in any case).
    fn completed(&self, expansions: &[Expansion]) -> Option<Found> {
        let typed: String = self.typed.iter().collect::<String>().to_lowercase();
        expansions
            .iter()
            .filter(|e| typed.ends_with(&e.keyword.to_lowercase()))
            .max_by_key(|e| e.keyword.chars().count())
            .map(|e| Found {
                erase: e.keyword.chars().count(),
                expansion: e.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::snippets::Snippet;

    fn settings(trigger: &str, keywords: &[&str]) -> SnippetSettings {
        SnippetSettings {
            trigger: trigger.into(),
            items: keywords
                .iter()
                .map(|k| Snippet {
                    name: format!("name {k}"),
                    keyword: (*k).into(),
                    text: format!("text for {k}"),
                })
                .collect(),
            ..SnippetSettings::default()
        }
    }

    fn key(c: char) -> Input {
        Input::Key {
            vk: c.to_ascii_uppercase() as u16,
            typed: Typed::Text(c.to_string()),
            ctrl: false,
            alt: false,
            win: false,
            window: 1,
        }
    }

    fn special(vk: u16, typed: Typed) -> Input {
        Input::Key {
            vk,
            typed,
            ctrl: false,
            alt: false,
            win: false,
            window: 1,
        }
    }

    fn type_text(t: &mut Typing, text: &str, ex: &[Expansion]) -> Option<Found> {
        text.chars().fold(None, |_, c| t.feed(key(c), ex))
    }

    #[test]
    fn only_trigger_keywords_expand() {
        let ex = expansions(&settings(";", &[";sig", "sig", ";", " ;addr "]));
        let keywords: Vec<&str> = ex.iter().map(|e| e.keyword.as_str()).collect();
        assert_eq!(keywords, [";sig", ";addr"]);
        // No trigger: every keyword.
        assert_eq!(expansions(&settings("", &["sig", ";x"])).len(), 2);
    }

    #[test]
    fn finds_a_keyword_as_it_is_typed() {
        let ex = expansions(&settings(";", &[";sig", ";addr"]));
        let mut t = Typing::default();
        assert_eq!(type_text(&mut t, "hello ;si", &ex), None);
        let found = t.feed(key('g'), &ex).unwrap();
        assert_eq!(found.erase, 4);
        assert_eq!(found.expansion.text, "text for ;sig");
        // Forgotten once used: another "g" doesn't fire again.
        assert_eq!(t.feed(key('g'), &ex), None);
        // Case doesn't matter (Caps Lock).
        assert!(type_text(&mut t, ";ADDR", &ex).is_some());
    }

    #[test]
    fn backspace_corrects_typos() {
        let ex = expansions(&settings(";", &[";sig"]));
        let mut t = Typing::default();
        type_text(&mut t, ";sx", &ex);
        t.feed(special(vk::BACK, Typed::Nothing), &ex);
        assert!(t.feed(key('i'), &ex).is_none());
        assert!(t.feed(key('g'), &ex).is_some());
    }

    #[test]
    fn caret_moves_forget_what_was_typed() {
        let ex = expansions(&settings(";", &[";sig"]));
        let resets = [
            Input::Click,
            special(0x25, Typed::Nothing),           // Left arrow
            special(0x0D, Typed::Text("\r".into())), // Enter
            Input::Key {
                vk: 0x41,
                typed: Typed::Text("a".into()),
                ctrl: true,
                alt: false,
                win: false,
                window: 1,
            }, // Ctrl+A
            Input::Key {
                vk: 0x53,
                typed: Typed::Text("s".into()),
                ctrl: false,
                alt: false,
                win: false,
                window: 2,
            }, // typing in another window
        ];
        for reset in resets {
            let mut t = Typing::default();
            type_text(&mut t, ";si", &ex);
            t.feed(reset, &ex);
            assert_eq!(t.feed(key('g'), &ex), None);
        }
    }

    #[test]
    fn modifiers_dead_keys_and_altgr_dont_interrupt() {
        let ex = expansions(&settings(";", &[";@x"]));
        let mut t = Typing::default();
        type_text(&mut t, ";", &ex);
        t.feed(special(0x10, Typed::Nothing), &ex); // Shift
        t.feed(special(0xDE, Typed::Dead), &ex); // an accent key
        let altgr_at = Input::Key {
            vk: 0x51,
            typed: Typed::Text("@".into()),
            ctrl: true,
            alt: true,
            win: false,
            window: 1,
        };
        assert!(t.feed(altgr_at, &ex).is_none());
        assert!(t.feed(key('x'), &ex).is_some());
    }

    #[test]
    fn longest_keyword_wins_and_memory_is_bounded() {
        let ex = expansions(&settings("", &["sig", ";sig"]));
        let mut t = Typing::default();
        let found = type_text(&mut t, "x;sig", &ex).unwrap();
        assert_eq!(found.expansion.keyword, ";sig");
        let mut t = Typing::default();
        type_text(&mut t, &"a".repeat(500), &ex);
        assert_eq!(t.typed.len(), REMEMBERED);
    }
}
