//! What one turn sends to the AI: the text (with any context blocks in
//! front) and pictures. Each provider turns it into its own shape: Claude
//! Code gets the PNG inline (base64), Codex gets the file's path.

use std::fmt;
use std::path::PathBuf;

/// A user message.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub images: Vec<ImageInput>,
}

/// A PNG on disk (in the launcher's data folder), read once so the message
/// can be built without touching the disk again.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub path: PathBuf,
    pub png: Vec<u8>,
}

impl Message {
    pub fn new(text: impl Into<String>, images: Vec<ImageInput>) -> Self {
        Self {
            text: text.into(),
            images,
        }
    }
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Self::new(text, Vec::new())
    }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Self::new(text, Vec::new())
    }
}

impl From<&String> for Message {
    fn from(text: &String) -> Self {
        Self::new(text.as_str(), Vec::new())
    }
}

// Messages can hold what you selected or a picture of your screen: debug
// output gives sizes only, so nothing private ends up in a log by accident.
impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("text_chars", &self.text.chars().count())
            .field("images", &self.images)
            .finish()
    }
}

impl fmt::Debug for ImageInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageInput")
            .field("path", &self.path)
            .field("bytes", &self.png.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_converts_and_debug_hides_content() {
        let m = Message::from("secret plan");
        assert_eq!(m.text, "secret plan");
        assert!(m.images.is_empty());
        let shown = format!(
            "{:?}",
            Message::new(
                "secret plan",
                vec![ImageInput {
                    path: "a.png".into(),
                    png: vec![1, 2, 3],
                }],
            )
        );
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("bytes: 3"), "{shown}");
    }
}
