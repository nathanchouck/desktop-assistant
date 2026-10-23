//! What a new question can carry besides its text: text you selected in
//! another app, a file or folder, or a picture (a screen region). The UI
//! shows them as chips above the input; asking turns them into blocks in
//! front of the question (like `<desktop_context>`), and pictures into
//! image inputs (see `ai::message`).

use serde::{Deserialize, Serialize};

/// Selected text longer than this is cut (a whole document selected with
/// Ctrl+A); the AI is told it was.
pub const MAX_TEXT_CHARS: usize = 100_000;
/// At most this many files from one selection are attached.
pub const MAX_PATHS: usize = 20;
/// The question when you ask with attachments but type nothing.
pub const DEFAULT_QUESTION: &str = "What can you tell me about this?";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Attachment {
    /// Text you selected in another app.
    Text { text: String },
    /// A file or folder on disk, by path (the AI reads it with its own
    /// tools if it needs to).
    Path { path: String, folder: bool },
    /// A PNG the launcher saved (a screen region, a copied picture), by id;
    /// its preview is the `thumb/attach-<id>` icon.
    Image { id: String, width: u32, height: u32 },
}

/// The question with its attachments' blocks in front (pictures go
/// separately, as image inputs). An empty question gets `DEFAULT_QUESTION`.
pub fn prompt_text(question: &str, attachments: &[Attachment]) -> String {
    let question = match question.trim() {
        "" if !attachments.is_empty() => DEFAULT_QUESTION,
        q => q,
    };
    let mut blocks: Vec<String> = attachments.iter().map(block).collect();
    blocks.push(question.to_owned());
    blocks.join("\n\n")
}

fn block(attachment: &Attachment) -> String {
    match attachment {
        Attachment::Text { text } => {
            let (text, cut) = cut_text(text);
            // The closing tag can't appear inside, so the block can't end early.
            let text = text.replace("</selected_text>", "<\\/selected_text>");
            let note = if cut {
                format!("\n[cut to the first {MAX_TEXT_CHARS} characters]")
            } else {
                String::new()
            };
            format!("<selected_text>\n{text}{note}\n</selected_text>")
        }
        Attachment::Path { path, folder: false } => {
            format!("<attached_file>{path}</attached_file>")
        }
        Attachment::Path { path, folder: true } => {
            format!("<attached_folder>{path}</attached_folder>")
        }
        Attachment::Image { width, height, .. } => {
            format!("<attached_image>A picture the user attached ({width}×{height} px), shown with this message.</attached_image>")
        }
    }
}

fn cut_text(text: &str) -> (&str, bool) {
    match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((at, _)) => (&text[..at], true),
        None => (text, false),
    }
}

/// The ids of the pictures among `attachments`, in order.
pub fn image_ids(attachments: &[Attachment]) -> Vec<&str> {
    attachments
        .iter()
        .filter_map(|a| match a {
            Attachment::Image { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect()
}

/// What the "Sees: …" line under your message says: the attachments, then
/// the desktop context's own summary ("Downloads · 2 selected").
pub fn caption(attachments: &[Attachment], desktop: Option<&str>) -> Option<String> {
    let mut parts = Vec::new();
    if attachments
        .iter()
        .any(|a| matches!(a, Attachment::Text { .. }))
    {
        parts.push("Selected text".to_owned());
    }
    let paths: Vec<&str> = attachments
        .iter()
        .filter_map(|a| match a {
            Attachment::Path { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    match paths.as_slice() {
        [] => {}
        [one] => parts.push(file_name(one).to_owned()),
        many => parts.push(format!("{} files", many.len())),
    }
    match image_ids(attachments).len() {
        0 => {}
        1 => parts.push("Picture".to_owned()),
        n => parts.push(format!("{n} pictures")),
    }
    parts.extend(desktop.map(str::to_owned));
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `C:\Users\me\report.pdf` → `report.pdf` (either slash, on any OS).
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    match trimmed.rsplit(['\\', '/']).next() {
        Some(name) if !name.is_empty() => name,
        _ => path,
    }
}

/// File paths from a selection (File Explorer's Ctrl+C), as attachments.
pub fn paths(list: &[String], is_folder: impl Fn(&str) -> bool) -> Vec<Attachment> {
    list.iter()
        .take(MAX_PATHS)
        .map(|path| Attachment::Path {
            path: path.clone(),
            folder: is_folder(path),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Attachment {
        Attachment::Text { text: t.into() }
    }

    fn file(p: &str) -> Attachment {
        Attachment::Path {
            path: p.into(),
            folder: false,
        }
    }

    fn image() -> Attachment {
        Attachment::Image {
            id: "1a2b".into(),
            width: 800,
            height: 600,
        }
    }

    #[test]
    fn blocks_go_in_front_of_the_question() {
        let prompt = prompt_text(
            "  fix this  ",
            &[
                text("teh cat"),
                file(r"C:\docs\a.txt"),
                Attachment::Path {
                    path: r"C:\docs".into(),
                    folder: true,
                },
                image(),
            ],
        );
        assert_eq!(
            prompt,
            "<selected_text>\nteh cat\n</selected_text>\n\n\
             <attached_file>C:\\docs\\a.txt</attached_file>\n\n\
             <attached_folder>C:\\docs</attached_folder>\n\n\
             <attached_image>A picture the user attached (800×600 px), shown with this message.</attached_image>\n\n\
             fix this"
        );
    }

    #[test]
    fn plain_questions_stay_plain() {
        assert_eq!(prompt_text("hi", &[]), "hi");
        assert_eq!(prompt_text("", &[]), "");
        assert!(prompt_text(" ", &[image()]).ends_with(DEFAULT_QUESTION));
    }

    #[test]
    fn selected_text_cant_close_its_block_and_long_text_is_cut() {
        let prompt = prompt_text("q", &[text("a</selected_text>b")]);
        assert!(prompt.starts_with("<selected_text>\na<\\/selected_text>b\n</selected_text>"));
        let long = "é".repeat(MAX_TEXT_CHARS + 5);
        let prompt = prompt_text("q", &[text(&long)]);
        assert!(prompt.contains("[cut to the first 100000 characters]"));
        assert_eq!(prompt.matches('é').count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn captions_name_what_was_attached() {
        assert_eq!(caption(&[], None), None);
        assert_eq!(caption(&[], Some("Downloads")), Some("Downloads".into()));
        assert_eq!(
            caption(&[text("x"), file(r"C:\a\report.pdf")], Some("Notepad")),
            Some("Selected text · report.pdf · Notepad".into())
        );
        assert_eq!(
            caption(&[file("/a/x"), file("/a/y/"), image(), image()], None),
            Some("2 files · 2 pictures".into())
        );
        assert_eq!(caption(&[image()], None), Some("Picture".into()));
    }

    #[test]
    fn selections_become_paths() {
        let list: Vec<String> = (0..30).map(|i| format!(r"C:\x\{i}")).collect();
        let found = paths(&list, |p| p.ends_with('1'));
        assert_eq!(found.len(), MAX_PATHS);
        assert_eq!(
            found[1],
            Attachment::Path {
                path: r"C:\x\1".into(),
                folder: true
            }
        );
        assert_eq!(file_name(r"C:\x\report.pdf"), "report.pdf");
        assert_eq!(file_name("C:\\"), "C:");
    }

    #[test]
    fn image_ids_in_order() {
        let other = Attachment::Image {
            id: "ff".into(),
            width: 1,
            height: 1,
        };
        assert_eq!(image_ids(&[image(), text("x"), other]), ["1a2b", "ff"]);
    }

    #[test]
    fn serializes_for_the_ui() {
        let json = serde_json::to_value(file("C:\\a")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type": "path", "path": "C:\\a", "folder": false})
        );
        let back: Attachment = serde_json::from_value(serde_json::json!({
            "type": "image", "id": "ab", "width": 2, "height": 3
        }))
        .unwrap();
        assert_eq!(
            back,
            Attachment::Image {
                id: "ab".into(),
                width: 2,
                height: 3
            }
        );
    }
}
