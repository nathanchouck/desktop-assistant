//! Turns a copy read from the clipboard into a history entry: decodes
//! images, spots repeats, saves image files. Runs on its own thread, so
//! the clipboard listener is never held up by a big picture.

use std::fs;

use super::content::{self, Captured, ClipKind};
use super::history::{History, HistoryError, NewClip};
use super::image::{self, ImageError};
use super::ClipboardSettings;

/// A copy, as the listener read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub content: Captured,
    /// The program it was copied in ("chrome"); may be empty.
    pub source: String,
}

/// What became of a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    Added,
    /// Already in the list: moved to the top.
    MovedUp,
    /// Nothing worth keeping (blank text, images turned off).
    Skipped,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Image(#[from] ImageError),
}

pub fn record(
    history: &History,
    capture: Capture,
    settings: &ClipboardSettings,
    now_ms: i64,
) -> Result<Recorded, RecordError> {
    let source = capture.source;
    let clip = match capture.content {
        Captured::Text(text) => {
            if text.trim().is_empty() || text.encode_utf16().count() > content::MAX_TEXT_UNITS {
                return Ok(Recorded::Skipped);
            }
            NewClip {
                kind: ClipKind::Text,
                hash: content::text_hash(&text),
                content: text,
                width: 0,
                height: 0,
                source,
            }
        }
        Captured::Files(paths) => {
            if paths.is_empty() {
                return Ok(Recorded::Skipped);
            }
            NewClip {
                kind: ClipKind::Files,
                hash: content::files_hash(&paths),
                content: paths.join("\n"),
                width: 0,
                height: 0,
                source,
            }
        }
        _ if !settings.keep_images => return Ok(Recorded::Skipped),
        image => return record_image(history, image, source, settings, now_ms),
    };
    let existed = history.bump_hash(&clip.hash, now_ms)?;
    if existed {
        return Ok(Recorded::MovedUp);
    }
    history.add(&clip, now_ms, settings.max_items)?;
    Ok(Recorded::Added)
}

/// Images: the PNG is kept as it came (or encoded from the bitmap), with a
/// thumbnail for the row.
fn record_image(
    history: &History,
    raw: Captured,
    source: String,
    settings: &ClipboardSettings,
    now_ms: i64,
) -> Result<Recorded, RecordError> {
    let (decoded, png) = match raw {
        Captured::Png(bytes) => (image::decode_png(&bytes)?, Some(bytes)),
        Captured::Dib(bytes) => (image::decode_dib(&bytes)?, None),
        Captured::Text(_) | Captured::Files(_) => return Ok(Recorded::Skipped),
    };
    let hash = content::image_hash(&decoded);
    if history.bump_hash(&hash, now_ms)? {
        return Ok(Recorded::MovedUp);
    }
    let file = match png {
        Some(bytes) => bytes,
        None => image::encode(&decoded)?,
    };
    let image_path = history.image_path(&hash);
    let thumb_path = history.thumb_path(&hash);
    for dir in [image_path.parent(), thumb_path.parent()]
        .into_iter()
        .flatten()
    {
        fs::create_dir_all(dir).map_err(HistoryError::from)?;
    }
    fs::write(&image_path, file).map_err(HistoryError::from)?;
    fs::write(&thumb_path, image::thumbnail(&decoded)?).map_err(HistoryError::from)?;
    history.add(
        &NewClip {
            kind: ClipKind::Image,
            content: String::new(),
            hash,
            width: decoded.width,
            height: decoded.height,
            source,
        },
        now_ms,
        settings.max_items,
    )?;
    Ok(Recorded::Added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::clipboard::history::Stored;
    use crate::platform::Image;

    fn setup() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().unwrap();
        let history =
            History::in_memory(dir.path().join("images"), dir.path().join("thumbs")).unwrap();
        (dir, history)
    }

    fn capture(content: Captured) -> Capture {
        Capture {
            content,
            source: "app".into(),
        }
    }

    #[test]
    fn records_text_and_files_once() {
        let (_dir, h) = setup();
        let s = ClipboardSettings::default();
        let text = || capture(Captured::Text("hello\nworld".into()));
        assert_eq!(record(&h, text(), &s, 1).unwrap(), Recorded::Added);
        assert_eq!(record(&h, text(), &s, 2).unwrap(), Recorded::MovedUp);
        let files = capture(Captured::Files(vec![r"C:\a.txt".into()]));
        assert_eq!(record(&h, files, &s, 3).unwrap(), Recorded::Added);
        assert_eq!(h.len(), 2);
        let blank = capture(Captured::Text(" \n ".into()));
        assert_eq!(record(&h, blank, &s, 4).unwrap(), Recorded::Skipped);
        let none = capture(Captured::Files(Vec::new()));
        assert_eq!(record(&h, none, &s, 5).unwrap(), Recorded::Skipped);
    }

    #[test]
    fn records_images_with_thumbnails_and_spots_repeats() {
        let (_dir, h) = setup();
        let s = ClipboardSettings::default();
        let picture = Image {
            width: 200,
            height: 100,
            rgba: (0..200 * 100 * 4).map(|i| (i % 251) as u8).collect(),
        };
        let png = image::encode(&picture).unwrap();
        assert_eq!(
            record(&h, capture(Captured::Png(png.clone())), &s, 1).unwrap(),
            Recorded::Added
        );
        let entry = h.with_entries(|e| e[0].clone());
        assert_eq!(entry.title, "Image 200 × 100");
        assert!(h.thumb_path(&entry.hash).exists());
        assert_eq!(
            h.stored(entry.id).unwrap(),
            Some(Stored::Image(h.image_path(&entry.hash)))
        );
        assert_eq!(fs::read(h.image_path(&entry.hash)).unwrap(), png);

        // The same picture as a bitmap (as another app might copy it) is a repeat.
        let mut dib = Vec::new();
        dib.extend(40u32.to_le_bytes());
        dib.extend(200i32.to_le_bytes());
        dib.extend((-100i32).to_le_bytes());
        dib.extend(1u16.to_le_bytes());
        dib.extend(32u16.to_le_bytes());
        dib.extend([0u8; 24]);
        for px in picture.rgba.chunks_exact(4) {
            dib.extend([px[2], px[1], px[0], px[3]]);
        }
        assert_eq!(
            record(&h, capture(Captured::Dib(dib)), &s, 2).unwrap(),
            Recorded::MovedUp
        );
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn images_can_be_turned_off_and_bad_ones_fail() {
        let (_dir, h) = setup();
        let off = ClipboardSettings {
            keep_images: false,
            ..ClipboardSettings::default()
        };
        let png = capture(Captured::Png(vec![1, 2, 3]));
        assert_eq!(record(&h, png.clone(), &off, 1).unwrap(), Recorded::Skipped);
        assert!(record(&h, png, &ClipboardSettings::default(), 1).is_err());
        assert!(h.is_empty());
    }
}
