//! Icons for apps, files and folders. Pulled from the OS the first time a
//! row needs one, cached on disk as PNG
//! (`%LOCALAPPDATA%\desktop-assistant\icons`) and in memory.
//! The UI loads them through the `icon` URI protocol registered in `lib.rs`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use crate::platform::{IconSource, Image, Shell};

/// Pixels per side. Rows draw icons at 24 px, so this covers 2x+ displays.
pub const ICON_SIZE: u32 = 64;

pub type Png = Arc<Vec<u8>>;

pub struct IconCache {
    dir: PathBuf,
    shell: Arc<Shell>,
    /// icon key → where to extract it from.
    sources: RwLock<HashMap<String, IconSource>>,
    memory: Mutex<HashMap<String, Png>>,
    /// Keys whose extraction failed this session; not retried per keystroke.
    failed: Mutex<HashSet<String>>,
}

/// Stable key for an icon identity (an app target, an extension, ...):
/// 16 hex chars (FNV-1a 64). Stable across runs and Rust versions, so the
/// disk cache stays valid.
pub fn key_for(target: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in target.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Keys come from the webview, so only accept exactly what `key_for` makes.
fn is_valid_key(key: &str) -> bool {
    key.len() == 16
        && key
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

impl IconCache {
    pub fn new(dir: PathBuf, shell: Arc<Shell>) -> Self {
        Self {
            dir,
            shell,
            sources: RwLock::new(HashMap::new()),
            memory: Mutex::new(HashMap::new()),
            failed: Mutex::new(HashSet::new()),
        }
    }

    /// Records where to get the icon for `key`. Keeps the first source seen,
    /// so many files sharing an extension icon don't churn the map.
    pub fn register(&self, key: &str, source: IconSource) {
        if let Ok(mut map) = self.sources.write() {
            map.entry(key.to_owned()).or_insert(source);
        }
    }

    /// The PNG for `key`, from memory, disk, or the OS (in that order).
    pub async fn png(&self, key: &str) -> Option<Png> {
        if !is_valid_key(key) {
            return None;
        }
        if let Some(hit) = self.memory.lock().ok()?.get(key) {
            return Some(Arc::clone(hit));
        }
        let path = self.dir.join(format!("{key}.png"));
        if let Ok(bytes) = fs::read(&path) {
            return Some(self.remember(key, bytes));
        }
        if self.failed.lock().ok()?.contains(key) {
            return None;
        }

        let source = self.sources.read().ok()?.get(key).cloned()?;
        let encoded = match self.shell.icon(source.clone(), ICON_SIZE).await {
            Ok(image) => encode_png(&image).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match encoded {
            Ok(bytes) => {
                let _ = fs::create_dir_all(&self.dir);
                if let Err(e) = fs::write(&path, &bytes) {
                    tracing::debug!("could not cache icon {key}: {e}");
                }
                Some(self.remember(key, bytes))
            }
            Err(e) => {
                tracing::debug!("no icon for {source:?}: {e}");
                if let Ok(mut failed) = self.failed.lock() {
                    failed.insert(key.to_owned());
                }
                None
            }
        }
    }

    /// Drops every cached icon so they're fetched fresh (used by Reindex).
    pub fn clear(&self) {
        if let Ok(mut m) = self.memory.lock() {
            m.clear();
        }
        if let Ok(mut f) = self.failed.lock() {
            f.clear();
        }
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "png") {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    fn remember(&self, key: &str, bytes: Vec<u8>) -> Png {
        let png = Arc::new(bytes);
        if let Ok(mut m) = self.memory.lock() {
            m.insert(key.to_owned(), Arc::clone(&png));
        }
        png
    }
}

/// The HTTP response for an `icon` protocol request: the PNG, or 404.
/// A PNG from the thumbnails folder, by name (letters, digits, `-`, `_`;
/// anything else is refused, so no path can escape the folder).
pub async fn thumbnail(dir: &std::path::Path, name: &str) -> Option<Png> {
    let safe = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return None;
    }
    let bytes = tokio::fs::read(dir.join(format!("{name}.png")))
        .await
        .ok()?;
    Some(Arc::new(bytes))
}

pub fn protocol_response(png: Option<Png>) -> tauri::http::Response<Vec<u8>> {
    let builder = tauri::http::Response::builder();
    let response = match png {
        Some(png) => builder
            .status(200)
            .header("Content-Type", "image/png")
            .header("Cache-Control", "max-age=604800")
            .body(png.to_vec()),
        None => builder.status(404).body(Vec::new()),
    };
    response.unwrap_or_default()
}

/// Windows hands back 32-bit BGRA with premultiplied alpha. Converts to
/// straight RGBA. Bitmaps with no alpha at all (every pixel 0) are opaque.
pub fn bgra_premultiplied_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let has_alpha = bgra.chunks_exact(4).any(|p| p[3] != 0);
    let mut out = Vec::with_capacity(bgra.len());
    for p in bgra.chunks_exact(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if !has_alpha {
            out.extend_from_slice(&[r, g, b, 255]);
        } else if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let un =
                |c: u8| ((u16::from(c) * 255 + u16::from(a) / 2) / u16::from(a)).min(255) as u8;
            out.extend_from_slice(&[un(r), un(g), un(b), a]);
        }
    }
    out
}

/// Window captures: BGRA whose alpha means nothing, as opaque RGBA.
pub fn bgra_to_opaque_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4)
        .flat_map(|p| [p[2], p[1], p[0], 255])
        .collect()
}

/// Shrinks `image` so neither side exceeds `max_side`, averaging the
/// pixels each new pixel covers. Smaller images come back unchanged.
pub fn fit_within(image: Image, max_side: u32) -> Image {
    let longest = image.width.max(image.height);
    if longest <= max_side || max_side == 0 {
        return image;
    }
    let scale = f64::from(max_side) / f64::from(longest);
    let width = ((f64::from(image.width) * scale).round() as u32).max(1);
    let height = ((f64::from(image.height) * scale).round() as u32).max(1);
    let (src_w, src_h) = (image.width as usize, image.height as usize);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let y0 = y * src_h / height as usize;
        let y1 = ((y + 1) * src_h / height as usize).max(y0 + 1);
        for x in 0..width as usize {
            let x0 = x * src_w / width as usize;
            let x1 = ((x + 1) * src_w / width as usize).max(x0 + 1);
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * src_w + sx) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += u32::from(image.rgba[i + c]);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            rgba.extend(sum.iter().map(|total| (total / count) as u8));
        }
    }
    Image {
        width,
        height,
        rgba,
    }
}

pub fn encode_png(image: &Image) -> Result<Vec<u8>, png::EncodingError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&image.rgba)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_are_opaque() {
        assert_eq!(bgra_to_opaque_rgba(&[10, 20, 30, 0]), [30, 20, 10, 255]);
    }

    #[test]
    fn shrinks_big_images_by_averaging() {
        // 4x2: left half black, right half white.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 0 } else { 255 };
                rgba.extend([v, v, v, 255]);
            }
        }
        let small = fit_within(
            Image {
                width: 4,
                height: 2,
                rgba: rgba.clone(),
            },
            2,
        );
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
        let same = fit_within(
            Image {
                width: 4,
                height: 2,
                rgba,
            },
            10,
        );
        assert_eq!((same.width, same.height), (4, 2));
    }

    #[test]
    fn keys_are_stable_and_valid() {
        let k = key_for("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
        assert_eq!(k, key_for("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert_ne!(k, key_for("notepad"));
        assert!(is_valid_key(&k));
    }

    #[test]
    fn rejects_unsafe_keys() {
        for bad in [
            "../../etc/passwd",
            "",
            "0123456789ABCDEF",
            "0123456789abcdef0",
        ] {
            assert!(!is_valid_key(bad), "{bad}");
        }
    }

    #[test]
    fn unpremultiplies_alpha() {
        // Half-transparent pure red, premultiplied: r = 255 * 0.5 ≈ 128.
        let rgba = bgra_premultiplied_to_rgba(&[0, 0, 128, 128]);
        assert_eq!(rgba, [255, 0, 0, 128]);
    }

    #[test]
    fn transparent_pixels_stay_transparent() {
        let rgba = bgra_premultiplied_to_rgba(&[0, 0, 0, 0, 0, 0, 255, 255]);
        assert_eq!(&rgba[..4], &[0, 0, 0, 0]);
        assert_eq!(&rgba[4..], &[255, 0, 0, 255]);
    }

    #[test]
    fn no_alpha_channel_means_opaque() {
        let rgba = bgra_premultiplied_to_rgba(&[10, 20, 30, 0]);
        assert_eq!(rgba, [30, 20, 10, 255]);
    }

    #[test]
    fn encodes_a_valid_png() {
        let image = Image {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 128],
        };
        let bytes = encode_png(&image).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[tokio::test]
    async fn serves_from_disk_cache() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_for("x");
        fs::write(dir.path().join(format!("{key}.png")), b"png").unwrap();
        let cache = IconCache::new(dir.path().into(), Arc::new(Shell::new()));
        assert_eq!(
            cache.png(&key).await.as_deref().map(Vec::as_slice),
            Some(&b"png"[..])
        );
    }

    #[tokio::test]
    async fn unknown_key_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IconCache::new(dir.path().into(), Arc::new(Shell::new()));
        assert!(cache.png(&key_for("never registered")).await.is_none());
    }
}
