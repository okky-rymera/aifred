//! Small text helpers that the tool servers share.

/// The first `max` characters of `s`, with a note at the end when it is longer.
/// The tool servers use it to keep their output small.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let c: String = s.chars().take(max).collect();
    format!("{c}\n[... shortened]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_keeps_short_text_and_marks_long_text() {
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("äbcd", 2), "äb\n[... shortened]");
    }
}
