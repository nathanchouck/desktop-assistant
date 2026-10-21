//! Copied images: decoding the clipboard's PNG and bitmap formats into
//! RGBA, and the small thumbnail a row shows.

use std::io::Cursor;

use crate::icons::{encode_png, fit_within};
use crate::platform::Image;

/// Thumbnails are this many pixels on their longest side (rows draw icons
/// at 24 px, so this stays sharp at 2x+).
pub const THUMB_SIZE: u32 = 64;
/// Bigger images (about a 10K screen) aren't kept.
const MAX_PIXELS: u64 = 50_000_000;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("not a PNG: {0}")]
    Png(String),
    #[error("unsupported bitmap: {0}")]
    Bitmap(&'static str),
    #[error("image too large ({0} × {1})")]
    TooLarge(u32, u32),
    #[error("could not encode PNG: {0}")]
    Encode(String),
}

fn check_size(width: u32, height: u32) -> Result<(), ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::Bitmap("empty image"));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(ImageError::TooLarge(width, height));
    }
    Ok(())
}

/// Decodes PNG bytes to 8-bit RGBA, whatever the PNG's own color type.
pub fn decode_png(bytes: &[u8]) -> Result<Image, ImageError> {
    let err = |e: png::DecodingError| ImageError::Png(e.to_string());
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_PIXELS as usize * 4,
        },
    );
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(err)?;
    let (width, height) = {
        let info = reader.info();
        (info.width, info.height)
    };
    check_size(width, height)?;
    let size = reader
        .output_buffer_size()
        .ok_or(ImageError::TooLarge(width, height))?;
    let mut buf = vec![0; size];
    let frame = reader.next_frame(&mut buf).map_err(err)?;
    buf.truncate(frame.buffer_size());
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(ImageError::Png("palette not expanded".into())),
    };
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(ImageError::Png("unexpected pixel data size".into()));
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

/// Decodes a device-independent bitmap (`CF_DIB` / `CF_DIBV5`: a
/// BITMAPINFOHEADER or V4/V5 header, then pixels) to RGBA. Handles the
/// 24- and 32-bit kinds the clipboard holds in practice; others are refused.
pub fn decode_dib(bytes: &[u8]) -> Result<Image, ImageError> {
    let u32_at = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or(ImageError::Bitmap("truncated header"))
    };
    let u16_at = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or(ImageError::Bitmap("truncated header"))
    };
    let header_size = u32_at(0)? as usize;
    if header_size < 40 {
        return Err(ImageError::Bitmap("old-style header"));
    }
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bit_count = u16_at(14)?;
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)?;
    if width <= 0 || height == 0 {
        return Err(ImageError::Bitmap("bad size"));
    }
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    check_size(w, h)?;
    // Rows are stored bottom-up unless the height is negative.
    let top_down = height < 0;

    let masks = match (compression, bit_count) {
        (BI_RGB, 24 | 32) => None,
        (BI_BITFIELDS, 32) => Some(if header_size >= 56 {
            // V4/V5 headers carry the masks (alpha too) inside the header.
            [u32_at(40)?, u32_at(44)?, u32_at(48)?, u32_at(52)?]
        } else {
            // A plain header is followed by three masks.
            [u32_at(40)?, u32_at(44)?, u32_at(48)?, 0]
        }),
        (BI_RGB | BI_BITFIELDS, _) => return Err(ImageError::Bitmap("not 24 or 32 bits")),
        _ => return Err(ImageError::Bitmap("compressed")),
    };
    let extra_masks = if compression == BI_BITFIELDS && header_size == 40 {
        12
    } else {
        0
    };
    let palette = colors_used as usize * 4;
    let offset = header_size + extra_masks + palette;

    let bytes_per_pixel = usize::from(bit_count / 8);
    let stride = (w as usize * bytes_per_pixel).div_ceil(4) * 4;
    let pixels = bytes
        .get(offset..offset + stride * h as usize)
        .ok_or(ImageError::Bitmap("truncated pixels"))?;

    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let row = if top_down { y } else { h as usize - 1 - y };
        let line = &pixels[row * stride..row * stride + w as usize * bytes_per_pixel];
        for px in line.chunks_exact(bytes_per_pixel) {
            match (bytes_per_pixel, masks) {
                (3, _) => rgba.extend([px[2], px[1], px[0], 255]),
                (_, None) => rgba.extend([px[2], px[1], px[0], px[3]]),
                (_, Some([r, g, b, a])) => {
                    let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
                    let alpha = if a == 0 { 255 } else { channel(v, a) };
                    rgba.extend([channel(v, r), channel(v, g), channel(v, b), alpha]);
                }
            }
        }
    }
    // 32-bit bitmaps often leave the alpha byte at 0, meaning "no alpha".
    if bit_count == 32 && rgba.chunks_exact(4).all(|p| p[3] == 0) {
        for p in rgba.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    Ok(Image {
        width: w,
        height: h,
        rgba,
    })
}

/// The bits of `value` under `mask`, scaled to 0..=255.
fn channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let raw = (value & mask) >> shift;
    if bits >= 8 {
        (raw >> (bits - 8)) as u8
    } else {
        let max = (1u32 << bits) - 1;
        ((raw * 255 + max / 2) / max) as u8
    }
}

/// The row icon for an image: a small PNG.
pub fn thumbnail(image: &Image) -> Result<Vec<u8>, ImageError> {
    let small = fit_within(image.clone(), THUMB_SIZE);
    encode_png(&small).map_err(|e| ImageError::Encode(e.to_string()))
}

/// The image as a PNG file.
pub fn encode(image: &Image) -> Result<Vec<u8>, ImageError> {
    encode_png(image).map_err(|e| ImageError::Encode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DIB with a 40-byte header (or 124 for V5) and the given rows.
    fn dib(
        width: i32,
        height: i32,
        bits: u16,
        compression: u32,
        extra: &[u8],
        pixels: &[u8],
    ) -> Vec<u8> {
        let header_size: u32 = if extra.len() == 84 { 124 } else { 40 };
        let mut b = Vec::new();
        b.extend(header_size.to_le_bytes());
        b.extend(width.to_le_bytes());
        b.extend(height.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(bits.to_le_bytes());
        b.extend(compression.to_le_bytes());
        b.extend((pixels.len() as u32).to_le_bytes());
        b.extend([0u8; 8]); // resolution
        b.extend(0u32.to_le_bytes()); // colors used
        b.extend(0u32.to_le_bytes()); // colors important
        b.extend(extra);
        b.extend(pixels);
        b
    }

    #[test]
    fn decodes_bottom_up_24_bit_with_row_padding() {
        // 1×2, bottom row first: blue (bottom), red (top). Rows pad to 4 bytes.
        let pixels = [255, 0, 0, 0, 0, 0, 255, 0];
        let image = decode_dib(&dib(1, 2, 24, BI_RGB, &[], &pixels)).unwrap();
        assert_eq!((image.width, image.height), (1, 2));
        assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn decodes_top_down_32_bit_and_treats_zero_alpha_as_opaque() {
        let pixels = [1, 2, 3, 0, 4, 5, 6, 0];
        let image = decode_dib(&dib(2, -1, 32, BI_RGB, &[], &pixels)).unwrap();
        assert_eq!(image.rgba, [3, 2, 1, 255, 6, 5, 4, 255]);
        // With some alpha set, it's kept.
        let pixels = [1, 2, 3, 128, 4, 5, 6, 0];
        let image = decode_dib(&dib(2, -1, 32, BI_RGB, &[], &pixels)).unwrap();
        assert_eq!(image.rgba, [3, 2, 1, 128, 6, 5, 4, 0]);
    }

    #[test]
    fn decodes_bitfields_after_a_plain_header() {
        let masks: Vec<u8> = [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF]
            .iter()
            .flat_map(|m| m.to_le_bytes())
            .collect();
        let pixels = 0x0011_2233u32.to_le_bytes();
        let image = decode_dib(&dib(1, 1, 32, BI_BITFIELDS, &masks, &pixels)).unwrap();
        assert_eq!(image.rgba, [0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn decodes_v5_with_alpha_mask() {
        // V5: masks R, G, B, A at the start of the extra 84 bytes.
        let mut extra = Vec::new();
        for m in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
            extra.extend(m.to_le_bytes());
        }
        extra.resize(84, 0);
        let pixels = 0x8011_2233u32.to_le_bytes();
        let image = decode_dib(&dib(1, 1, 32, BI_BITFIELDS, &extra, &pixels)).unwrap();
        assert_eq!(image.rgba, [0x11, 0x22, 0x33, 0x80]);
    }

    #[test]
    fn refuses_what_it_cannot_read() {
        assert!(decode_dib(&[1, 2, 3]).is_err());
        assert_eq!(
            decode_dib(&dib(1, 1, 8, BI_RGB, &[], &[0; 4])),
            Err(ImageError::Bitmap("not 24 or 32 bits"))
        );
        assert_eq!(
            decode_dib(&dib(1, 1, 32, 4, &[], &[0; 4])),
            Err(ImageError::Bitmap("compressed"))
        );
        assert_eq!(
            decode_dib(&dib(4, 4, 32, BI_RGB, &[], &[0; 8])),
            Err(ImageError::Bitmap("truncated pixels"))
        );
        assert!(matches!(
            decode_dib(&dib(100_000, 100_000, 32, BI_RGB, &[], &[])),
            Err(ImageError::TooLarge(..))
        ));
    }

    #[test]
    fn scales_narrow_masks() {
        assert_eq!(channel(0b1_1111, 0b1_1111), 255);
        assert_eq!(channel(0, 0b1_1111), 0);
        assert_eq!(channel(0xABCD, 0xFFFF), 0xAB);
        assert_eq!(channel(5, 0), 0);
    }

    #[test]
    fn png_round_trip_and_gray_pngs() {
        let image = Image {
            width: 2,
            height: 1,
            rgba: vec![10, 20, 30, 40, 50, 60, 70, 255],
        };
        let png = encode(&image).unwrap();
        assert_eq!(decode_png(&png).unwrap(), image);

        let mut gray = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut gray, 2, 1);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 200]).unwrap();
        }
        let decoded = decode_png(&gray).unwrap();
        assert_eq!(decoded.rgba, [0, 0, 0, 255, 200, 200, 200, 255]);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn thumbnails_are_small() {
        let image = Image {
            width: 300,
            height: 150,
            rgba: vec![128; 300 * 150 * 4],
        };
        let thumb = decode_png(&thumbnail(&image).unwrap()).unwrap();
        assert_eq!((thumb.width, thumb.height), (THUMB_SIZE, THUMB_SIZE / 2));
    }
}
