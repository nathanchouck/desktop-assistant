use serde::{Deserialize, Serialize};

/// What a result is. The UI picks a fallback glyph and a label from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    App,
    File,
    Folder,
    Command,
    AskAi,
    /// A calculation or conversion; always ranked first.
    Answer,
    /// A clipboard history entry.
    Clip,
    Snippet,
    Emoji,
    /// A web link or web search.
    Link,
    Process,
    Window,
    /// A past AI conversation.
    Conversation,
    /// A user script.
    Script,
}

/// Something the user can do with a result. The first action is the default
/// (Enter); the others have their own keys (see `ui/src/lib/keys.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Open,
    RunAsAdmin,
    RevealInExplorer,
    CopyPath,
    AskAi,
    Run,
    /// Copy the result's text (a calculation, a clip, an emoji).
    Copy,
    /// Paste the result's text into the window you were in.
    Paste,
    /// Remove the result (a clip, a conversation).
    Delete,
    /// End a running program (the process killer).
    End,
    /// Pin or unpin the result.
    Pin,
    /// Ask the AI about this result (a file, a clip).
    AskAbout,
}

/// One row in the results list.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct ResultItem {
    /// Stable across runs: "app:<id>", "file:<path>", "cmd:<name>". Frecency
    /// is keyed on this, so never derive it from anything that changes.
    pub id: String,
    pub kind: ItemKind,
    pub title: String,
    pub subtitle: String,
    /// What the row shows as its icon: a key for the `icon` protocol (app
    /// and file icons, `thumb/<name>` images), or `glyph:<name>` (a
    /// built-in symbol, see `ui/src/lib/glyphs.ts`), `text:<chars>` (an
    /// emoji or symbol drawn as text), `color:#rrggbb` (a swatch). Rows
    /// without one show a glyph for their kind.
    pub icon: Option<String>,
    /// Match score in 0..=1 from the provider; the ranker adjusts it. Used
    /// only for ordering in Rust, so it is not sent to the UI.
    #[serde(skip)]
    #[specta(skip)]
    pub score: f32,
    pub actions: Vec<Action>,
    /// Short text at the right of the row ("Pinned", "2 min ago").
    pub accessory: Option<String>,
    /// Asks before the default action runs: this question shows in the
    /// footer and Enter again confirms ("Restart your PC?").
    pub confirm: Option<String>,
}

impl ResultItem {
    pub fn new(id: impl Into<String>, kind: ItemKind, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            title: title.into(),
            subtitle: String::new(),
            icon: None,
            score: 0.0,
            actions: Vec::new(),
            accessory: None,
            confirm: None,
        }
    }

    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_score(mut self, score: f32) -> Self {
        self.score = score;
        self
    }

    pub fn with_actions(mut self, actions: impl IntoIterator<Item = Action>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    pub fn with_accessory(mut self, accessory: impl Into<String>) -> Self {
        self.accessory = Some(accessory.into());
        self
    }

    pub fn with_confirm(mut self, question: impl Into<String>) -> Self {
        self.confirm = Some(question.into());
        self
    }
}

/// `icon` value for a built-in symbol (see `ui/src/lib/glyphs.ts`).
pub fn glyph(name: &str) -> String {
    format!("glyph:{name}")
}

/// `icon` value that draws `chars` (an emoji or symbol) as the icon.
pub fn text_icon(chars: &str) -> String {
    format!("text:{chars}")
}

/// `icon` value for a color swatch; `hex` is `#rrggbb`.
pub fn color_icon(hex: &str) -> String {
    format!("color:{hex}")
}

/// `icon` value for a PNG in the thumbnails folder (`AppPaths::thumbnails_dir`).
/// `name` may only use letters, digits, `-` and `_` (without `.png`).
pub fn thumb_icon(name: &str) -> String {
    format!("thumb/{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_fills_every_field() {
        let item = ResultItem::new("calc:1", ItemKind::Answer, "4")
            .with_subtitle("2 + 2")
            .with_icon(glyph("calculator"))
            .with_score(1.0)
            .with_actions([Action::Copy, Action::Paste])
            .with_accessory("Enter to copy")
            .with_confirm("Sure?");
        assert_eq!(item.subtitle, "2 + 2");
        assert_eq!(item.icon.as_deref(), Some("glyph:calculator"));
        assert_eq!(item.actions, [Action::Copy, Action::Paste]);
        assert_eq!(item.accessory.as_deref(), Some("Enter to copy"));
        assert_eq!(item.confirm.as_deref(), Some("Sure?"));
    }

    #[test]
    fn icon_helpers() {
        assert_eq!(text_icon("🔥"), "text:🔥");
        assert_eq!(color_icon("#ff8800"), "color:#ff8800");
        assert_eq!(thumb_icon("clip-12"), "thumb/clip-12");
    }
}
