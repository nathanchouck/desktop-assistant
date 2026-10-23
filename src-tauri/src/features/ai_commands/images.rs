//! Pictures attached to a question (a screen region, a copied picture):
//! saved as PNG files in `ai-attachments` in the data folder, with a
//! preview in the thumbnails folder (`thumb/attach-<id>`). Claude gets the
//! file's bytes inline; Codex reads the file itself, so it stays until the
//! next start or until it's a day old.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::attachments::Attachment;
use crate::features::clipboard::content::Captured;
use crate::features::clipboard::image::{decode_dib, decode_png};
use crate::icons::{encode_png, fit_within};
use crate::platform::Image;

/// The folder in the data folder.
pub const DIR_NAME: &str = "ai-attachments";
/// Pictures are shrunk to this many pixels on the longest side, the size
/// the AIs work at anyway (the launcher's screenshot tool uses it too).
const MAX_SIDE: u32 = 1568;
/// The preview the chip and the compose view show.
const PREVIEW_SIDE: u32 = 320;
/// Thumbnails of ours start with this (the folder is shared).
const THUMB_PREFIX: &str = "attach-";
/// Files older than this are deleted when a new picture is saved.
pub const KEEP_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// `thumb_icon` name of a picture's preview.
pub fn thumb_name(id: &str) -> String {
    format!("{THUMB_PREFIX}{id}")
}

/// A fresh id: the time in ms and a counter, in hex (so it's a safe file
/// name and a valid thumbnail name).
pub fn new_id() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) & 0xff;
    format!("{ms:x}{n:02x}")
}

/// Ids come back from the UI: only lowercase hex, so no path can escape
/// the folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where picture `id` is saved, if the id is valid.
pub fn path(dir: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| dir.join(format!("{id}.png")))
}

/// Saves `image` (shrunk to `MAX_SIDE`) and its preview. Blocks: call off
/// the async runtime.
pub fn save(image: Image, dir: &Path, thumbs: &Path) -> Result<Attachment, String> {
    let id = new_id();
    let image = fit_within(image, MAX_SIDE);
    let png = encode_png(&image).map_err(|e| e.to_string())?;
    let preview =
        encode_png(&fit_within(image.clone(), PREVIEW_SIDE)).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(thumbs).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{id}.png")), png).map_err(|e| e.to_string())?;
    fs::write(thumbs.join(format!("{}.png", thumb_name(&id))), preview)
        .map_err(|e| e.to_string())?;
    Ok(Attachment::Image {
        id,
        width: image.width,
        height: image.height,
    })
}

/// The picture in a clipboard copy: a PNG, a bitmap, or a single copied
/// PNG file (some screenshot tools copy the file they saved).
pub fn from_clipboard(captured: &Captured) -> Option<Image> {
    match captured {
        Captured::Png(bytes) => decode_png(bytes).ok(),
        Captured::Dib(bytes) => decode_dib(bytes).ok(),
        Captured::Files(paths) => match paths.as_slice() {
            [one] if one.to_lowercase().ends_with(".png") => decode_png(&fs::read(one).ok()?).ok(),
            _ => None,
        },
        Captured::Text(_) => None,
    }
}

/// Deletes pictures (and their previews) older than `max_age`; with
/// `Duration::ZERO`, all of them (at startup, when no AI can still be
/// reading one).
pub fn sweep(dir: &Path, thumbs: &Path, max_age: Duration) {
    let now = SystemTime::now();
    let old = |path: &Path| {
        max_age.is_zero()
            || fs::metadata(path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).unwrap_or_default() > max_age)
    };
    let mut removed = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for path in entries.flatten().map(|e| e.path()) {
            let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".png"))
                .filter(|id| valid_id(id))
            else {
                continue;
            };
            if old(&path) && fs::remove_file(&path).is_ok() {
                let _ = fs::remove_file(thumbs.join(format!("{}.png", thumb_name(id))));
                removed += 1;
            }
        }
    }
    if removed > 0 {
        tracing::debug!("removed {removed} old attached pictures");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            rgba: vec![200; (width * height * 4) as usize],
        }
    }

    #[test]
    fn ids_are_fresh_hex() {
        let (a, b) = (new_id(), new_id());
        assert_ne!(a, b);
        assert!(valid_id(&a) && valid_id(&b), "{a} {b}");
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id("ABC"));
        assert!(!valid_id(&"a".repeat(33)));
        assert_eq!(path(Path::new("/d"), "..\\x"), None);
        assert_eq!(
            path(Path::new("/d"), "1f"),
            Some(Path::new("/d").join("1f.png"))
        );
    }

    #[test]
    fn saves_a_shrunk_picture_and_its_preview() {
        let dir = tempfile::tempdir().unwrap();
        let (files, thumbs) = (dir.path().join("a"), dir.path().join("t"));
        let saved = save(picture(3136, 100), &files, &thumbs).unwrap();
        let Attachment::Image { id, width, height } = saved else {
            panic!("not an image: {saved:?}");
        };
        assert_eq!((width, height), (1568, 50));
        let png = fs::read(path(&files, &id).unwrap()).unwrap();
        assert_eq!(decode_png(&png).unwrap().width, 1568);
        let preview = fs::read(thumbs.join(format!("attach-{id}.png"))).unwrap();
        assert_eq!(decode_png(&preview).unwrap().width, PREVIEW_SIDE);
    }

    #[test]
    fn sweeping_removes_ours_only() {
        let dir = tempfile::tempdir().unwrap();
        let (files, thumbs) = (dir.path().join("a"), dir.path().join("t"));
        let Attachment::Image { id, .. } = save(picture(4, 4), &files, &thumbs).unwrap() else {
            panic!("not an image");
        };
        fs::write(files.join("notes.txt"), "keep").unwrap();
        fs::write(thumbs.join("clip-1.png"), "keep").unwrap();
        sweep(&files, &thumbs, KEEP_FOR);
        assert!(path(&files, &id).unwrap().exists(), "new pictures stay");
        sweep(&files, &thumbs, Duration::ZERO);
        assert!(!path(&files, &id).unwrap().exists());
        assert!(!thumbs.join(format!("attach-{id}.png")).exists());
        assert!(files.join("notes.txt").exists());
        assert!(thumbs.join("clip-1.png").exists());
    }

    #[test]
    fn reads_pictures_from_the_clipboard() {
        let png = encode_png(&picture(2, 3)).unwrap();
        assert_eq!(
            from_clipboard(&Captured::Png(png.clone())).map(|i| i.height),
            Some(3)
        );
        assert_eq!(from_clipboard(&Captured::Text("x".into())), None);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Screenshot.PNG");
        fs::write(&file, &png).unwrap();
        let files = Captured::Files(vec![file.display().to_string()]);
        assert_eq!(from_clipboard(&files).map(|i| i.width), Some(2));
        let two = Captured::Files(vec!["a.png".into(), "b.png".into()]);
        assert_eq!(from_clipboard(&two), None);
    }
}
