//! What a copy holds, and the pure helpers around it: parsing the raw
//! clipboard formats, the hashes that spot repeats, and row text.

use crate::icons::key_for;
use crate::platform::Image;

/// Text clips longer than this (in UTF-16 units, as Windows stores them)
/// aren't kept: a whole log file isn't something you paste back from here.
pub const MAX_TEXT_UNITS: usize = 2 * 1024 * 1024;
/// Row titles are cut to this many characters.
const TITLE_CHARS: usize = 80;
/// The search filter matches this much of a text clip.
const PREVIEW_CHARS: usize = 1000;

/// A copy as read from the clipboard, before images are decoded (that
/// happens off the clipboard, so it isn't held open meanwhile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Captured {
    Text(String),
    /// Copied files and folders (from File Explorer), as full paths.
    Files(Vec<String>),
    /// The bytes of a PNG file (the "PNG" format browsers and Snipping Tool add).
    Png(Vec<u8>),
    /// A device-independent bitmap (`CF_DIBV5` / `CF_DIB`): header, then pixels.
    Dib(Vec<u8>),
}

/// What an entry is, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    Text,
    Image,
    Files,
}

impl ClipKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            "files" => Some(Self::Files),
            _ => None,
        }
    }
}

/// Text from `CF_UNICODETEXT` bytes: UTF-16LE up to the first NUL.
pub fn text_from_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Paths from `CF_HDROP` bytes: a `DROPFILES` header (offset of the list
/// at 0, "wide" flag at 16), then NUL-separated names ending in an empty one.
pub fn files_from_dropfiles(bytes: &[u8]) -> Vec<String> {
    let read_u32 = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (Some(offset), Some(wide)) = (read_u32(0), read_u32(16)) else {
        return Vec::new();
    };
    let Some(list) = bytes.get(offset as usize..) else {
        return Vec::new();
    };
    let names: Vec<String> = if wide != 0 {
        let units: Vec<u16> = list
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        units
            .split(|&u| u == 0)
            .take_while(|name| !name.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    } else {
        // ANSI names: only ASCII is safe to read without the code page.
        list.split(|&b| b == 0)
            .take_while(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect()
    };
    names
}

/// Identifies an entry's content, so copying the same thing again moves
/// the old entry up instead of adding another. Images hash their pixels,
/// so the same picture encoded differently is still the same.
pub fn text_hash(text: &str) -> String {
    key_for(&format!("text\n{text}"))
}

pub fn files_hash(paths: &[String]) -> String {
    key_for(&format!("files\n{}", paths.join("\n")))
}

pub fn image_hash(image: &Image) -> String {
    // FNV-1a over the size and pixels, the same function as `key_for`.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let size = [image.width.to_le_bytes(), image.height.to_le_bytes()].concat();
    for &byte in size.iter().chain(image.rgba.iter()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The row title for a text clip: its first non-blank line, trimmed and cut.
pub fn text_title(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    match line {
        Some(line) => cut(line, TITLE_CHARS),
        None => "(blank)".into(),
    }
}

/// "report.pdf", or "report.pdf and 2 more".
pub fn files_title(paths: &[String]) -> String {
    let name = |p: &String| {
        p.trim_end_matches(['\\', '/'])
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(p)
            .to_owned()
    };
    match paths {
        [] => "No files".into(),
        [one] => cut(&name(one), TITLE_CHARS),
        [first, rest @ ..] => format!("{} and {} more", cut(&name(first), 60), rest.len()),
    }
}

pub fn image_title(width: u32, height: u32) -> String {
    format!("Image {width} × {height}")
}

/// What the search filter looks at: the start of a text clip.
pub fn preview(text: &str) -> String {
    text.chars().take(PREVIEW_CHARS).collect()
}

fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago".
pub fn age(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", secs / 60),
        3_600..86_400 => format!("{} h ago", secs / 3_600),
        86_400..172_800 => "yesterday".into(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

/// A program's name from its path: `C:\...\chrome.exe` → `chrome`.
pub fn program_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match file.len().checked_sub(4) {
        Some(cut)
            if file
                .get(cut..)
                .is_some_and(|ext| ext.eq_ignore_ascii_case(".exe")) =>
        {
            file[..cut].to_owned()
        }
        _ => file.to_owned(),
    }
}

/// Whether copies made in `app` (a program name such as "KeePassXC" or
/// "keepassxc.exe") are ignored by the `ignore` list.
pub fn is_ignored(app: &str, ignore: &[String]) -> bool {
    let stem = |s: &str| {
        let s = s.trim();
        let lower = s.to_lowercase();
        match lower.strip_suffix(".exe") {
            Some(stem) => stem.to_owned(),
            None => lower,
        }
    };
    let app = stem(app);
    !app.is_empty() && ignore.iter().any(|entry| stem(entry) == app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn reads_unicode_text_up_to_the_nul() {
        let mut bytes = utf16("héllo ✓");
        bytes.extend([0, 0, b'x', 0]);
        assert_eq!(text_from_utf16(&bytes), "héllo ✓");
        assert_eq!(text_from_utf16(&utf16("no terminator")), "no terminator");
    }

    fn dropfiles(names: &[&str], wide: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(20u32.to_le_bytes()); // pFiles
        bytes.extend([0u8; 8]); // pt
        bytes.extend(0u32.to_le_bytes()); // fNC
        bytes.extend(u32::from(wide).to_le_bytes()); // fWide
        for name in names {
            if wide {
                bytes.extend(utf16(name));
                bytes.extend([0, 0]);
            } else {
                bytes.extend(name.as_bytes());
                bytes.push(0);
            }
        }
        bytes.extend(if wide { &[0u8, 0][..] } else { &[0u8][..] });
        bytes
    }

    #[test]
    fn reads_file_lists() {
        let names = [r"C:\Users\me\Désktop\a.txt", r"C:\Users\me\b"];
        assert_eq!(files_from_dropfiles(&dropfiles(&names, true)), names);
        assert_eq!(
            files_from_dropfiles(&dropfiles(&[r"C:\x.txt"], false)),
            [r"C:\x.txt"]
        );
        assert!(files_from_dropfiles(&[1, 2, 3]).is_empty());
        let mut bad_offset = dropfiles(&names, true);
        bad_offset[0] = 0xFF;
        assert!(files_from_dropfiles(&bad_offset).is_empty());
    }

    #[test]
    fn hashes_tell_kinds_and_contents_apart() {
        assert_eq!(text_hash("a"), text_hash("a"));
        assert_ne!(text_hash("a"), text_hash("b"));
        assert_ne!(text_hash("a"), files_hash(&["a".into()]));
        let image = |rgba: Vec<u8>, width| Image {
            width,
            height: 1,
            rgba,
        };
        let one = image_hash(&image(vec![1, 2, 3, 4, 5, 6, 7, 8], 2));
        assert_eq!(one.len(), 16);
        assert_eq!(one, image_hash(&image(vec![1, 2, 3, 4, 5, 6, 7, 8], 2)));
        assert_ne!(one, image_hash(&image(vec![1, 2, 3, 4, 5, 6, 7, 9], 2)));
        // Hashes become file names and icon keys: safe characters only.
        assert!(one.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn titles() {
        assert_eq!(text_title("\n\n  first line  \nsecond"), "first line");
        assert_eq!(text_title(" \n\t"), "(blank)");
        let long = "word ".repeat(40);
        let title = text_title(&long);
        assert_eq!(title.chars().count(), 80);
        assert!(title.ends_with("word…"), "{title}");
        assert_eq!(files_title(&[r"C:\a\report.pdf".into()]), "report.pdf");
        assert_eq!(
            files_title(&[r"C:\a\Photos\".into(), r"C:\b.png".into(), "c".into()]),
            "Photos and 2 more"
        );
        assert_eq!(image_title(1920, 1080), "Image 1920 × 1080");
    }

    #[test]
    fn ages() {
        let now = 10_000_000_000;
        let ago = |secs: i64| age(now, now - secs * 1000);
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(-5), "just now");
        assert_eq!(ago(5 * 60), "5 min ago");
        assert_eq!(ago(3 * 3600 + 5), "3 h ago");
        assert_eq!(ago(30 * 3600), "yesterday");
        assert_eq!(ago(4 * 86_400 + 10), "4 days ago");
    }

    #[test]
    fn ignore_list_matches_program_names() {
        let ignore = ["1Password".to_owned(), "keepassxc.exe".to_owned()];
        assert!(is_ignored("1Password.exe", &ignore));
        assert!(is_ignored("KeePassXC", &ignore));
        assert!(is_ignored("KEEPASSXC.EXE", &ignore));
        assert!(!is_ignored("KeePass.exe", &ignore));
        assert!(!is_ignored("", &ignore));
    }

    #[test]
    fn program_names() {
        assert_eq!(
            program_name(r"C:\Program Files\Google\chrome.exe"),
            "chrome"
        );
        assert_eq!(program_name(r"C:\Windows\EXPLORER.EXE"), "EXPLORER");
        assert_eq!(program_name("Code"), "Code");
        assert_eq!(program_name(".exe"), "");
    }

    #[test]
    fn kinds_round_trip() {
        for kind in [ClipKind::Text, ClipKind::Image, ClipKind::Files] {
            assert_eq!(ClipKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ClipKind::parse("video"), None);
    }
}
