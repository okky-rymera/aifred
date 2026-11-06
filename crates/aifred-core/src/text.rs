//! Small text helpers that the tool servers and the AI calls share.

/// The first `max` characters of `s`, with a note at the end when it is longer.
/// The tool servers use it to keep their output small.
pub fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let c: String = s.chars().take(max).collect();
    format!("{c}\n[... shortened]")
}

/// `s` on one line, with at most `max` characters. A longer text ends with "…".
pub fn one_line(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let c: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", c.trim_end())
}

/// The JSON object in an AI answer, also when the AI put a code fence or
/// words around it: the text from the first "{" to the last "}".
pub fn json_object(answer: &str) -> Option<&str> {
    let start = answer.find('{')?;
    let end = answer.rfind('}').filter(|&e| e > start)?;
    Some(&answer[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_keeps_short_text_and_marks_long_text() {
        assert_eq!(cut("abc", 3), "abc");
        assert_eq!(cut("äbcd", 2), "äb\n[... shortened]");
    }

    #[test]
    fn one_line_joins_lines_and_cuts_long_text() {
        assert_eq!(one_line(" a\n b  c ", 10), "a b c");
        assert_eq!(one_line("abcdef", 4), "abc…");
        assert_eq!(one_line("abc", 0), "…", "a limit of 0 does not fail");
    }

    #[test]
    fn json_object_finds_the_object_in_other_text() {
        assert_eq!(
            json_object("```json\n{\"a\": {}}\n```"),
            Some("{\"a\": {}}")
        );
        assert_eq!(json_object("no json"), None);
        assert_eq!(json_object("} then {"), None);
    }
}
