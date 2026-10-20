use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    App,
    File,
    Folder,
    Command,
    AskAi,
}

/// Something the user can do with a result. The first action is the default
/// (Enter); the rest appear in the action menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Open,
    RunAsAdmin,
    RevealInExplorer,
    CopyPath,
    AskAi,
    Run,
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
    /// Match score in 0..=1 from the provider; the ranker adjusts it. Used
    /// only for ordering in Rust, so it is not sent to the UI.
    #[serde(skip)]
    #[specta(skip)]
    pub score: f32,
    pub actions: Vec<Action>,
}
