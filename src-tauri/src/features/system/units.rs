//! Sizes and counts as people read them ("1.4 GB", "3 items"). Shared by
//! the Recycle Bin row and the process list.

/// Bytes in Windows' units (1 KB = 1024 bytes), one decimal under 10.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        // "1.0 GB" reads oddly; drop a zero decimal.
        let text = format!("{value:.1}");
        let text = text.strip_suffix(".0").unwrap_or(&text);
        format!("{text} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// "1 item", "3 items".
pub fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0), "0 bytes");
        assert_eq!(format_size(1023), "1023 bytes");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(340 * 1024 * 1024), "340 MB");
        assert_eq!(format_size(1_503_238_553), "1.4 GB");
        assert_eq!(format_size(12 * 1024 * 1024 * 1024), "12 GB");
        assert_eq!(format_size(5 * 1024u64.pow(4)), "5 TB");
        assert_eq!(format_size(2048 * 1024u64.pow(4)), "2048 TB");
    }

    #[test]
    fn counts() {
        assert_eq!(count(1, "item", "items"), "1 item");
        assert_eq!(count(0, "item", "items"), "0 items");
        assert_eq!(count(12, "process", "processes"), "12 processes");
    }
}
