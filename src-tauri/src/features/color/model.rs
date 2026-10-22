//! Colors: reading what you type (`#ff8800`, `#f80`, `rgb(255, 136, 0)`,
//! `hsl(32, 100%, 50%)`) and writing hex, rgb() and hsl(). Pure.

use serde::{Deserialize, Serialize};

/// How a color is written when it's copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ColorFormat {
    /// `#ff8800`
    Hex,
    /// `rgb(255, 136, 0)`
    Rgb,
    /// `hsl(32, 100%, 50%)`
    Hsl,
}

impl ColorFormat {
    pub const ALL: [Self; 3] = [Self::Hex, Self::Rgb, Self::Hsl];

    pub fn label(self) -> &'static str {
        match self {
            Self::Hex => "Hex",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `#rrggbb`, lower case (also what `color_icon` takes).
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn format(self, format: ColorFormat) -> String {
        match format {
            ColorFormat::Hex => self.hex(),
            ColorFormat::Rgb => format!("rgb({}, {}, {})", self.r, self.g, self.b),
            ColorFormat::Hsl => {
                let (h, s, l) = self.hsl();
                format!("hsl({h}, {s}%, {l}%)")
            }
        }
    }

    /// Hue in degrees (0..360), saturation and lightness in percent, all
    /// rounded as CSS tools show them.
    pub fn hsl(self) -> (u16, u8, u8) {
        let [r, g, b] = [self.r, self.g, self.b].map(|c| f64::from(c) / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        if d == 0.0 {
            return (0, 0, percent(l));
        }
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        ((h.round() as u16) % 360, percent(s), percent(l))
    }

    /// From hue in degrees and saturation and lightness in 0..=1.
    pub fn from_hsl(h: f64, s: f64, l: f64) -> Self {
        let h = h.rem_euclid(360.0);
        let s = s.clamp(0.0, 1.0);
        let l = l.clamp(0.0, 1.0);
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
        let m = l - c / 2.0;
        let (r, g, b) = match h {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self::new(byte(r), byte(g), byte(b))
    }
}

fn percent(v: f64) -> u8 {
    (v * 100.0).round().clamp(0.0, 100.0) as u8
}

/// A color typed in search, and the format it was typed in.
pub fn parse(text: &str) -> Option<(Rgb, ColorFormat)> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        return parse_hex(hex).map(|c| (c, ColorFormat::Hex));
    }
    let lower = text.to_ascii_lowercase();
    if let Some(args) = function_args(&lower, "rgb") {
        return parse_rgb(&args).map(|c| (c, ColorFormat::Rgb));
    }
    if let Some(args) = function_args(&lower, "hsl") {
        return parse_hsl(&args).map(|c| (c, ColorFormat::Hsl));
    }
    None
}

/// `#rgb` or `#rrggbb` (without the `#`).
fn parse_hex(hex: &str) -> Option<Rgb> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    match hex.len() {
        3 => Some(Rgb::new(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
        6 => Some(Rgb::new(pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}

/// The three arguments of `name(a, b, c)` (commas or spaces between them).
fn function_args(text: &str, name: &str) -> Option<Vec<String>> {
    let inner = text
        .strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let args: Vec<String> = inner
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|a| !a.is_empty())
        .map(str::to_owned)
        .collect();
    (args.len() == 3).then_some(args)
}

fn parse_rgb(args: &[String]) -> Option<Rgb> {
    let channel = |a: &String| a.parse::<u8>().ok();
    Some(Rgb::new(
        channel(&args[0])?,
        channel(&args[1])?,
        channel(&args[2])?,
    ))
}

fn parse_hsl(args: &[String]) -> Option<Rgb> {
    let hue: f64 = args[0]
        .strip_suffix("deg")
        .unwrap_or(&args[0])
        .parse()
        .ok()?;
    let share = |a: &String| -> Option<f64> {
        let v: f64 = a.strip_suffix('%')?.parse().ok()?;
        (0.0..=100.0).contains(&v).then_some(v / 100.0)
    };
    if !hue.is_finite() {
        return None;
    }
    Some(Rgb::from_hsl(hue, share(&args[1])?, share(&args[2])?))
}

/// Parses a stored `#rrggbb`.
pub fn from_hex(hex: &str) -> Option<Rgb> {
    parse_hex(hex.strip_prefix('#')?).filter(|_| hex.len() == 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORANGE: Rgb = Rgb::new(255, 136, 0);

    #[test]
    fn writes_each_format() {
        assert_eq!(ORANGE.format(ColorFormat::Hex), "#ff8800");
        assert_eq!(ORANGE.format(ColorFormat::Rgb), "rgb(255, 136, 0)");
        assert_eq!(ORANGE.format(ColorFormat::Hsl), "hsl(32, 100%, 50%)");
        assert_eq!(Rgb::new(0, 0, 0).format(ColorFormat::Hsl), "hsl(0, 0%, 0%)");
        assert_eq!(
            Rgb::new(255, 255, 255).format(ColorFormat::Hsl),
            "hsl(0, 0%, 100%)"
        );
        assert_eq!(
            Rgb::new(128, 128, 128).format(ColorFormat::Hsl),
            "hsl(0, 0%, 50%)"
        );
    }

    #[test]
    fn hsl_round_trips_the_primaries() {
        for (rgb, hsl) in [
            (Rgb::new(255, 0, 0), (0, 100, 50)),
            (Rgb::new(0, 255, 0), (120, 100, 50)),
            (Rgb::new(0, 0, 255), (240, 100, 50)),
            (Rgb::new(255, 0, 255), (300, 100, 50)),
            (Rgb::new(0, 128, 128), (180, 100, 25)),
        ] {
            assert_eq!(rgb.hsl(), hsl, "{rgb:?}");
            let (h, s, l) = hsl;
            assert_eq!(
                Rgb::from_hsl(f64::from(h), f64::from(s) / 100.0, f64::from(l) / 100.0),
                rgb
            );
        }
    }

    #[test]
    fn reads_hex() {
        assert_eq!(parse("#ff8800"), Some((ORANGE, ColorFormat::Hex)));
        assert_eq!(parse("#FF8800"), Some((ORANGE, ColorFormat::Hex)));
        assert_eq!(parse("#f80"), Some((ORANGE, ColorFormat::Hex)));
        assert_eq!(parse("#ff880"), None);
        assert_eq!(parse("#ff88001"), None);
        assert_eq!(parse("#gg8800"), None);
        assert_eq!(parse("#é8"), None);
        assert_eq!(parse("ff8800"), None, "a # is required");
        assert_eq!(parse("#"), None);
    }

    #[test]
    fn reads_rgb_and_hsl() {
        let rgb = Some((ORANGE, ColorFormat::Rgb));
        assert_eq!(parse("rgb(255, 136, 0)"), rgb);
        assert_eq!(parse("rgb(255,136,0)"), rgb);
        assert_eq!(parse("RGB(255 136 0)"), rgb);
        assert_eq!(parse("rgb (255, 136, 0)"), rgb);
        assert_eq!(parse("rgb(256, 0, 0)"), None);
        assert_eq!(parse("rgb(1, 2)"), None);
        assert_eq!(parse("rgb(1, 2, 3, 4)"), None);
        assert_eq!(parse("rgb(1, 2, 3"), None);
        assert_eq!(parse("rgb(-1, 2, 3)"), None);

        let hsl = parse("hsl(32, 100%, 50%)").unwrap();
        assert_eq!(hsl.1, ColorFormat::Hsl);
        assert_eq!(hsl.0.hsl(), (32, 100, 50));
        assert_eq!(parse("hsl(32deg 100% 50%)").unwrap().0, hsl.0);
        assert_eq!(parse("hsl(32, 100, 50)"), None, "percent signs needed");
        assert_eq!(parse("hsl(32, 120%, 50%)"), None);
        assert_eq!(parse("hsl(nan, 10%, 50%)"), None);
        assert_eq!(parse("hsl(inf, 10%, 50%)"), None);
        // Hues wrap around.
        assert_eq!(
            parse("hsl(-120, 100%, 50%)").unwrap().0,
            Rgb::new(0, 0, 255)
        );
        assert_eq!(parse("hsl(480, 100%, 50%)").unwrap().0, Rgb::new(0, 255, 0));
    }

    #[test]
    fn ordinary_text_isnt_a_color() {
        for text in ["red", "rgb", "hsl", "#hashtag", "rgb()", "colors", "#1"] {
            assert_eq!(parse(text), None, "{text}");
        }
    }

    #[test]
    fn stored_hex() {
        assert_eq!(from_hex("#ff8800"), Some(ORANGE));
        assert_eq!(from_hex("#f80"), None);
        assert_eq!(from_hex("ff8800"), None);
    }
}
