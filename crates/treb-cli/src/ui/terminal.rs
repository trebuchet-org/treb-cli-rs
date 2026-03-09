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

/// Strips all ANSI escape sequences from a string, returning only the visible text.
///
/// Handles standard SGR codes (`\x1b[...m`), 256-color codes (`\x1b[38;5;Nm`),
/// and combined parameter codes (`\x1b[0;1;31m`).
#[allow(dead_code)]
pub fn strip_ansi_codes(s: &str) -> String {
    console::strip_ansi_codes(s).into_owned()
}

/// Returns the display width of a string, stripping ANSI escape codes before measuring.
///
/// Uses unicode-width for accurate measurement of wide characters.
#[allow(dead_code)]
pub fn display_width(s: &str) -> usize {
    let stripped = strip_ansi_codes(s);
    UnicodeWidthStr::width(stripped.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_width_returns_positive() {
        assert!(terminal_width() > 0);
    }

    // -- strip_ansi_codes tests --

    #[test]
    fn strip_ansi_basic_sgr() {
        assert_eq!(strip_ansi_codes("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_no_codes() {
        assert_eq!(strip_ansi_codes("no codes"), "no codes");
    }

    #[test]
    fn strip_ansi_empty_string() {
        assert_eq!(strip_ansi_codes(""), "");
    }

    #[test]
    fn strip_ansi_nested_multiple_codes() {
        assert_eq!(strip_ansi_codes("\x1b[1m\x1b[31mtext\x1b[0m"), "text");
    }

    #[test]
    fn strip_ansi_multiple_parameters() {
        // 256-color code
        assert_eq!(strip_ansi_codes("\x1b[38;5;196mred256\x1b[0m"), "red256");
        // Combined parameters
        assert_eq!(strip_ansi_codes("\x1b[0;1;31mbold_red\x1b[0m"), "bold_red");
    }

    #[test]
    fn strip_ansi_mixed_plain_and_codes() {
        assert_eq!(
            strip_ansi_codes("pre\x1b[32m-mid-\x1b[0mpost"),
            "pre-mid-post"
        );
    }

    #[test]
    fn strip_ansi_only_codes_no_text() {
        assert_eq!(strip_ansi_codes("\x1b[31m\x1b[0m"), "");
    }

    // -- display_width tests --

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
