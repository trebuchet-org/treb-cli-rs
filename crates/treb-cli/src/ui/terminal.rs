//! Terminal width detection and display width measurement utilities.

use std::borrow::Cow;

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
pub fn strip_ansi_codes(s: &str) -> Cow<'_, str> {
    console::strip_ansi_codes(s)
}

/// Returns the display width of a string using Unicode-aware column measurement.
///
/// Uses the `unicode-width` crate for accurate measurement of wide characters
/// (e.g., CJK ideographs, certain emoji). Callers must strip ANSI escape codes
/// before calling this function — it does **not** strip them itself.
#[allow(dead_code)]
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
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
    fn strip_ansi_no_codes_returns_borrowed() {
        assert!(matches!(strip_ansi_codes("no codes"), Cow::Borrowed("no codes")));
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
        assert_eq!(strip_ansi_codes("pre\x1b[32m-mid-\x1b[0mpost"), "pre-mid-post");
    }

    #[test]
    fn strip_ansi_only_codes_no_text() {
        assert_eq!(strip_ansi_codes("\x1b[31m\x1b[0m"), "");
    }

    #[test]
    fn strip_ansi_codes_returns_owned_when_modified() {
        assert!(matches!(
            strip_ansi_codes("\x1b[31mred\x1b[0m"),
            Cow::Owned(ref text) if text == "red"
        ));
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
    fn display_width_check_mark_with_variation_selector() {
        // ✔︎ = U+2714 (HEAVY CHECK MARK) + U+FE0E (text variation selector)
        // go-runewidth returns 1 for this combination
        assert_eq!(display_width("✔\u{FE0E}"), 1);
    }

    #[test]
    fn display_width_hourglass_emoji() {
        // ⏳ = U+23F3 (HOURGLASS WITH FLOWING SAND) — East Asian Width "W"
        // go-runewidth returns 2
        assert_eq!(display_width("⏳"), 2);
    }

    #[test]
    fn display_width_cjk_wide_char() {
        // CJK ideograph — always 2 columns wide
        assert_eq!(display_width("漢"), 2);
        assert_eq!(display_width("漢字"), 4);
    }

    #[test]
    fn display_width_mixed_ascii_and_unicode() {
        // "Name ✔︎" = 5 (Name ) + 1 (✔︎) = 6
        assert_eq!(display_width("Name \u{2714}\u{FE0E}"), 6);
    }

    #[test]
    fn display_width_box_drawing_chars() {
        // Box-drawing characters used in tree rendering — each 1 column wide
        assert_eq!(display_width("├─ "), 3);
        assert_eq!(display_width("└─ "), 3);
        assert_eq!(display_width("│"), 1);
    }
}
