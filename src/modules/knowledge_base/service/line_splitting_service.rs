pub struct RawLine {
    pub line_number: i32,
    pub line_text: String,
    pub line_chars: i32,
    pub cumulative_chars: i64,
}

/// Split text into lines with character counting metadata.
///
/// Lines are 1-indexed. `cumulative_chars` is the sum of all `line_chars`
/// from line 1 through the current line (inclusive).
#[must_use]
pub fn split_lines(text: &str) -> Vec<RawLine> {
    let mut lines = Vec::new();
    let mut cumulative: i64 = 0;

    for (idx, line) in text.split('\n').enumerate() {
        let line_chars = i32::try_from(line.chars().count()).unwrap_or(i32::MAX);
        cumulative += i64::from(line_chars);

        lines.push(RawLine {
            line_number: i32::try_from(idx.saturating_add(1)).unwrap_or(i32::MAX),
            line_text: line.to_string(),
            line_chars,
            cumulative_chars: cumulative,
        });
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_lines_empty_text() {
        let lines = split_lines("");
        // "".split('\n') yields one element (the empty string itself)
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_text, "");
        assert_eq!(lines[0].line_chars, 0);
    }

    #[test]
    fn split_lines_single_line() {
        let lines = split_lines("hello world");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_number, 1);
        assert_eq!(lines[0].line_text, "hello world");
        assert_eq!(lines[0].line_chars, 11);
        assert_eq!(lines[0].cumulative_chars, 11);
    }

    #[test]
    fn split_lines_multiple_lines() {
        let lines = split_lines("aaa\nbbb\nccc");
        assert_eq!(lines.len(), 3);

        assert_eq!(lines[0].line_number, 1);
        assert_eq!(lines[0].line_text, "aaa");
        assert_eq!(lines[0].line_chars, 3);
        assert_eq!(lines[0].cumulative_chars, 3);

        assert_eq!(lines[1].line_number, 2);
        assert_eq!(lines[1].line_text, "bbb");
        assert_eq!(lines[1].line_chars, 3);
        assert_eq!(lines[1].cumulative_chars, 6);

        assert_eq!(lines[2].line_number, 3);
        assert_eq!(lines[2].line_text, "ccc");
        assert_eq!(lines[2].line_chars, 3);
        assert_eq!(lines[2].cumulative_chars, 9);
    }

    #[test]
    fn split_lines_trailing_newline_produces_empty_last_line() {
        let lines = split_lines("foo\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line_text, "foo");
        assert_eq!(lines[1].line_text, "");
        assert_eq!(lines[1].line_chars, 0);
        assert_eq!(lines[1].cumulative_chars, 3);
    }

    #[test]
    fn split_lines_consecutive_newlines() {
        let lines = split_lines("a\n\nb");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].line_text, "a");
        assert_eq!(lines[1].line_text, "");
        assert_eq!(lines[2].line_text, "b");
    }

    #[test]
    fn split_lines_unicode_chars() {
        let lines = split_lines("你好世界");
        assert_eq!(lines[0].line_chars, 4);
        assert_eq!(lines[0].cumulative_chars, 4);
    }

    #[test]
    fn split_lines_cumulative_chars_monotonic() {
        let text = "line one\nline two is longer\nshort\nfinal line here";
        let lines = split_lines(text);
        let mut prev_cum = 0i64;
        for line in &lines {
            assert!(
                line.cumulative_chars >= prev_cum,
                "cumulative_chars should be monotonically increasing"
            );
            prev_cum = line.cumulative_chars;
        }
    }
}
