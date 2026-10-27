//! Reading and writing the system clipboard: for pasting history entries
//! back, and for snippets (which borrow the clipboard to paste and then
//! put back what was there).
//!
//! Writing goes through `arboard` (the crate behind the clipboard plugin),
//! which can put images (PNG + bitmap) and file lists on the clipboard and
//! mark a write as private; the plugin itself only does text.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use super::content::Captured;
use super::image;
use crate::platform::Image;

/// What to put on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    Files(Vec<String>),
    Image(Image),
}

/// The clipboard change we made last (Windows' sequence number; 0 for
/// none), so the history doesn't record our own writes: pasting an entry
/// moves it up by itself.
static OWN_WRITE: AtomicU32 = AtomicU32::new(0);
/// Writes in progress. The listener hears of a write as soon as the
/// clipboard closes, which can be before `OWN_WRITE` is set.
static WRITING: AtomicU32 = AtomicU32::new(0);
/// The longest the listener waits for one of our writes to finish.
const WRITE_WAIT: Duration = Duration::from_secs(2);

/// Whether the clipboard change numbered `sequence` was ours.
pub fn is_own_write(sequence: u32) -> bool {
    sequence != 0 && OWN_WRITE.load(Ordering::SeqCst) == sequence
}

/// Whether the clipboard's current change was ours, once any write of
/// ours still in progress has finished. `sequence` reads the current
/// change's number.
pub fn is_own_change(sequence: impl Fn() -> u32) -> bool {
    let started = Instant::now();
    while WRITING.load(Ordering::SeqCst) > 0 && started.elapsed() < WRITE_WAIT {
        std::thread::sleep(Duration::from_millis(5));
    }
    is_own_write(sequence())
}

/// Counts a write as in progress until dropped.
struct Writing;

impl Writing {
    fn begin() -> Self {
        WRITING.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Windows numbers every clipboard change; 0 elsewhere.
pub fn sequence() -> u32 {
    #[cfg(windows)]
    {
        super::win::sequence_number()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Puts `content` on the clipboard and returns the change's sequence
/// number. `private` keeps text out of clipboard managers and Windows'
/// clipboard history (Win+V): for snippet text that's only there to be
/// pasted. Blocks briefly (it may encode an image): call off the async
/// runtime.
pub fn write(content: &Content, private: bool) -> Result<u32, String> {
    let _writing = Writing::begin();
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let set = clipboard.set();
    #[cfg(windows)]
    let set = if private {
        use arboard::SetExtWindows;
        set.exclude_from_monitoring()
    } else {
        set
    };
    #[cfg(not(windows))]
    let _ = private;
    let written = match content {
        Content::Text(text) => set.text(text.as_str()),
        Content::Files(paths) => set.file_list(paths),
        Content::Image(image) => set.image(arboard::ImageData {
            width: image.width as usize,
            height: image.height as usize,
            bytes: Cow::Borrowed(&image.rgba),
        }),
    };
    written.map_err(|e| e.to_string())?;
    let sequence = sequence();
    OWN_WRITE.store(sequence, Ordering::SeqCst);
    Ok(sequence)
}

/// What's on the clipboard now, if it's text, files or an image.
pub fn read() -> Option<Captured> {
    #[cfg(windows)]
    {
        super::win::read_now()
    }
    #[cfg(not(windows))]
    {
        let mut clipboard = arboard::Clipboard::new().ok()?;
        clipboard.get().text().ok().map(Captured::Text)
    }
}

/// Everything that was on the clipboard, to put back after borrowing it
/// (snippets, the selection shortcut). On Windows that's every format, so
/// an Excel range or formatted text comes back as it was.
pub struct Saved {
    #[cfg(windows)]
    snapshot: super::win::Snapshot,
    #[cfg(not(windows))]
    text: Option<String>,
}

impl Saved {
    /// The text that was on the clipboard, if any (for `{clipboard}`).
    pub fn text(&self) -> Option<String> {
        #[cfg(windows)]
        {
            self.snapshot.text()
        }
        #[cfg(not(windows))]
        {
            self.text.clone()
        }
    }
}

/// Saves what's on the clipboard now; `None` if it couldn't be read. May
/// wait for the app that copied to render it: call off the async runtime.
pub fn save() -> Option<Saved> {
    #[cfg(windows)]
    {
        super::win::snapshot().map(|snapshot| Saved { snapshot })
    }
    #[cfg(not(windows))]
    {
        Some(Saved { text: read_text() })
    }
}

/// Puts saved content back, kept out of clipboard histories (it was there
/// already) and not recorded again by ours.
pub fn put_back(saved: &Saved) -> Result<(), String> {
    let _writing = Writing::begin();
    #[cfg(windows)]
    {
        let sequence = super::win::restore(&saved.snapshot).map_err(|e| e.message())?;
        OWN_WRITE.store(sequence, Ordering::SeqCst);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        match &saved.text {
            Some(text) => write(&Content::Text(text.clone()), true).map(|_| ()),
            None => Ok(()),
        }
    }
}

/// The clipboard's text, if it holds text.
pub fn read_text() -> Option<String> {
    match read()? {
        Captured::Text(text) => Some(text),
        _ => None,
    }
}

/// Something read from the clipboard, ready to be written back.
pub fn to_content(captured: Captured) -> Option<Content> {
    match captured {
        Captured::Text(text) => Some(Content::Text(text)),
        Captured::Files(paths) => Some(Content::Files(paths)),
        Captured::Png(bytes) => image::decode_png(&bytes).ok().map(Content::Image),
        Captured::Dib(bytes) => image::decode_dib(&bytes).ok().map(Content::Image),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_writes_are_recognized_by_number() {
        assert!(!is_own_write(0));
        OWN_WRITE.store(42, Ordering::SeqCst);
        assert!(is_own_write(42));
        assert!(!is_own_write(43));
        assert!(is_own_change(|| 42));
        assert!(!is_own_change(|| 7));
    }

    #[test]
    fn captured_content_converts_back() {
        assert_eq!(
            to_content(Captured::Text("hi".into())),
            Some(Content::Text("hi".into()))
        );
        assert_eq!(
            to_content(Captured::Files(vec!["C:\\a".into()])),
            Some(Content::Files(vec!["C:\\a".into()]))
        );
        let picture = Image {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        };
        let png = image::encode(&picture).unwrap();
        assert_eq!(
            to_content(Captured::Png(png)),
            Some(Content::Image(picture))
        );
        assert_eq!(to_content(Captured::Dib(vec![0; 3])), None);
    }
}
