//! Human-readable formatting. Kept in one place so the UI, the CLI and the
//! Markdown export all render identical numbers.

const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// Format a byte count in binary units, e.g. `698.2 GiB`.
///
/// Bytes are rendered without a decimal because "512.0 B" reads like a bug.
pub fn bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut value = n as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Group digits with commas, e.g. `1,842,109`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let lead = digits.len() % 3;
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && i % 3 == lead {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Percentage with one decimal, e.g. `59.1%`. A zero whole yields `0.0%`
/// rather than NaN.
pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0.0%".to_string();
    }
    format!("{:.1}%", part as f64 * 100.0 / whole as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_render_in_binary_units() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(1024 * 1024 * 1024), "1.0 GiB");
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1_842_109), "1,842,109");
    }

    #[test]
    fn percent_of_zero_is_not_nan() {
        assert_eq!(percent(5, 0), "0.0%");
        assert_eq!(percent(1, 2), "50.0%");
    }
}
