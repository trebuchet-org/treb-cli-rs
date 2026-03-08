//! Terminal width detection and display width measurement utilities.

use console::Term;
use unicode_width::UnicodeWidthStr;

/// Returns the current terminal width in columns.
///
/// Falls back to 80 if the terminal width cannot be determined (e.g., non-TTY).
#[allow(dead_code)]
pub fn terminal_width() -> usize {
    Term::stdout().size_checked().map(|(_, cols)| cols as usize).unwrap_or(80)
}

/// Returns the display width of a string, stripping ANSI escape codes before measuring.
///
/// Uses unicode-width for accurate measurement of wide characters.
#[allow(dead_code)]
pub fn display_width(s: &str) -> usize {
    let stripped = console::strip_ansi_codes(s);
    UnicodeWidthStr::width(stripped.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_width_returns_positive() {
        assert!(terminal_width() > 0);
    }

    #[test]
    fn display_width_plain_ascii() {
        assert_eq!(display_width("hello"), 5);
    }

    #[test]
    fn display_width_empty_string() {
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn display_width_strips_ansi() {
        assert_eq!(display_width("\x1b[31mhello\x1b[0m"), 5);
    }

    #[test]
    fn display_width_complex_ansi() {
        // Bold + color + underline
        assert_eq!(display_width("\x1b[1;31;4mtest\x1b[0m"), 4);
    }

    #[test]
    fn display_width_mixed_ansi_and_plain() {
        assert_eq!(display_width("pre\x1b[32m-mid-\x1b[0mpost"), 12);
    }
}
