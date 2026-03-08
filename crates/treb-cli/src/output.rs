//! Shared output formatting utilities for CLI commands.
//!
//! Provides consistent output patterns: pretty JSON, UTF-8 tables with bold
//! headers, aligned key-value pair printing, and stage progress indicators.

use comfy_table::{Attribute, Cell, ContentArrangement, Table};
use owo_colors::OwoColorize;
use serde::Serialize;

use crate::ui::{color, emoji};

/// Print a value as pretty-printed JSON to stdout.
///
/// Recursively sorts all object keys so that output is deterministic
/// regardless of the underlying map type (e.g. `HashMap`).
pub fn print_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let json_value = sort_json_keys(serde_json::to_value(value)?);
    let json = serde_json::to_string_pretty(&json_value)?;
    println!("{json}");
    Ok(())
}

/// Recursively sort all object keys in a JSON value.
fn sort_json_keys(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in entries {
                sorted.insert(k, sort_json_keys(v));
            }
            serde_json::Value::Object(sorted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sort_json_keys).collect())
        }
        other => other,
    }
}

/// Build a UTF-8 table with bold headers.
pub fn build_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.load_preset(comfy_table::presets::UTF8_FULL);

    let header_cells: Vec<Cell> =
        headers.iter().map(|h| Cell::new(h).add_attribute(Attribute::Bold)).collect();
    table.set_header(header_cells);

    table
}

/// Print a table to stdout.
pub fn print_table(table: &Table) {
    println!("{table}");
}

/// Print key-value pairs with right-padded keys for alignment.
pub fn print_kv(pairs: &[(&str, &str)]) {
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in pairs {
        println!("{:>width$}:  {}", key, value, width = max_key_len);
    }
}

/// Print key-value pairs with right-padded keys for alignment to stderr.
pub fn eprint_kv(pairs: &[(&str, &str)]) {
    let max_key_len = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (key, value) in pairs {
        eprintln!("{:>width$}:  {}", key, value, width = max_key_len);
    }
}

/// Truncate an address to `0xABCD...EFGH` format (first 4 + last 4 hex chars).
pub fn truncate_address(address: &str) -> String {
    if address.len() >= 10 {
        format!("{}...{}", &address[..6], &address[address.len() - 4..])
    } else {
        address.to_string()
    }
}

/// Format a number with comma-separated thousands (e.g., `1234567` → `"1,234,567"`).
pub fn format_gas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Format a stage progress message with emoji prefix.
///
/// Returns `"emoji message"` with the message styled using the [`color::STAGE`]
/// palette when color is enabled, or plain text when disabled.
pub fn format_stage(emoji: &str, message: &str) -> String {
    if color::is_color_enabled() {
        format!("{} {}", emoji, message.style(color::STAGE))
    } else {
        format!("{} {}", emoji, message)
    }
}

/// Print a stage progress message to stderr with emoji prefix.
///
/// Outputs `"emoji styled_message\n"` to stderr. The message is styled with
/// [`color::STAGE`] when color is enabled.
pub fn print_stage(emoji: &str, message: &str) {
    eprintln!("{}", format_stage(emoji, message));
}

/// Format a warning banner message with emoji prefix.
///
/// Returns `"emoji message"` with the message styled using [`color::WARNING`]
/// when color is enabled, or plain text when disabled.
pub fn format_warning_banner(emoji: &str, message: &str) -> String {
    if color::is_color_enabled() {
        format!("{} {}", emoji, message.style(color::WARNING))
    } else {
        format!("{} {}", emoji, message)
    }
}

/// Print a warning banner to stdout with emoji prefix and warning styling.
pub fn print_warning_banner(emoji: &str, message: &str) {
    println!("{}", format_warning_banner(emoji, message));
}

/// Print a JSON error object to stderr: `{"error":"<message>"}`.
///
/// Used by the top-level error handler when `--json` is set so that
/// automation consumers receive a machine-parseable error on stderr.
pub fn print_json_error(message: &str) {
    let obj = serde_json::json!({ "error": message });
    // Unwrap is safe — the value is always a valid JSON object.
    eprintln!("{}", serde_json::to_string(&obj).unwrap());
}

// ---------------------------------------------------------------------------
// Go-matching format helpers (render/helpers.go)
// ---------------------------------------------------------------------------

/// Extract the message portion after the last `: ` separator, or return
/// the full input if no separator is found.
fn extract_message(input: &str) -> &str {
    input.rsplit_once(": ").map_or(input, |(_, msg)| msg)
}

/// Capitalize the first character of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let mut result = String::with_capacity(s.len());
            for upper in c.to_uppercase() {
                result.push(upper);
            }
            result.push_str(chars.as_str());
            result
        }
    }
}

/// Extract a tag name from a message like `tag 'v1' already exists`.
fn extract_tag_name(msg: &str) -> Option<&str> {
    let start = msg.find('\'')?;
    let end = msg[start + 1..].find('\'')?;
    Some(&msg[start + 1..start + 1 + end])
}

/// Format a warning message matching Go `render/helpers.go` `FormatWarning`.
///
/// Splits on `: ` and takes the last segment. Special-cases tag "already exists"
/// and "does not exist" messages. Falls back to `⚠️  {msg}` in yellow.
#[allow(dead_code)]
pub fn format_warning(message: &str) -> String {
    let msg = extract_message(message);

    let formatted = if msg.contains("already exists") {
        if let Some(tag) = extract_tag_name(msg) {
            format!("Deployment already has tag '{tag}'")
        } else {
            msg.to_string()
        }
    } else if msg.contains("does not exist") {
        if let Some(tag) = extract_tag_name(msg) {
            format!("Deployment doesn't have tag '{tag}'")
        } else {
            msg.to_string()
        }
    } else {
        msg.to_string()
    };

    if color::is_color_enabled() {
        format!("{}  {}", emoji::WARNING, formatted.style(color::WARNING))
    } else {
        format!("{}  {}", emoji::WARNING, formatted)
    }
}

/// Format an error message matching Go `render/helpers.go` `FormatError`.
///
/// Splits on `: ` and takes the last segment, capitalizes the first letter,
/// and returns `❌ {msg}` in red.
#[allow(dead_code)]
pub fn format_error(message: &str) -> String {
    let msg = extract_message(message);
    let capitalized = capitalize_first(msg);

    if color::is_color_enabled() {
        format!("{} {}", emoji::CROSS, capitalized.style(color::RED))
    } else {
        format!("{} {}", emoji::CROSS, capitalized)
    }
}

/// Format a success message matching Go `render/helpers.go` `FormatSuccess`.
///
/// Returns `✅ {msg}` in green.
#[allow(dead_code)]
pub fn format_success(message: &str) -> String {
    if color::is_color_enabled() {
        format!("{} {}", emoji::CHECK, message.style(color::GREEN))
    } else {
        format!("{} {}", emoji::CHECK, message)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::*;

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().expect("env test lock poisoned")
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: Serialized by env_lock() in tests that mutate env vars.
            unsafe { std::env::set_var(key, value) };
            Self { key, original }
        }

        fn unset(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: Serialized by env_lock() in tests that mutate env vars.
            unsafe { std::env::remove_var(key) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::set_var(self.key, value) };
                }
                None => {
                    // SAFETY: Serialized by env_lock() in tests that mutate env vars.
                    unsafe { std::env::remove_var(self.key) };
                }
            }
        }
    }

    #[test]
    fn format_gas_zero() {
        assert_eq!(format_gas(0), "0");
    }

    #[test]
    fn format_gas_small() {
        assert_eq!(format_gas(999), "999");
    }

    #[test]
    fn format_gas_thousands() {
        assert_eq!(format_gas(1234), "1,234");
    }

    #[test]
    fn format_gas_millions() {
        assert_eq!(format_gas(1_234_567), "1,234,567");
    }

    #[test]
    fn format_stage_with_color_enabled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let result = format_stage("\u{1f528}", "Compiling...");
        assert!(result.contains("\u{1f528}"), "should contain emoji");
        assert!(result.contains("Compiling..."), "should contain message");
        assert!(result.contains("\x1b["), "should contain ANSI codes when color enabled");
    }

    #[test]
    fn format_stage_with_color_disabled() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true); // override_disabled = true -> color off

        let result = format_stage("\u{1f528}", "Compiling...");
        assert_eq!(result, "\u{1f528} Compiling...");
        assert!(!result.contains("\x1b["), "should not contain ANSI codes when color disabled");

        // Restore
        owo_colors::set_override(true);
    }

    // -- format_warning tests --

    #[test]
    fn format_warning_tag_already_exists() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_warning("tag error: tag 'v1' already exists");
        assert!(
            result.contains("Deployment already has tag 'v1'"),
            "expected tag-already-exists message, got: {result}"
        );
        assert!(result.starts_with(emoji::WARNING), "should start with warning emoji");

        owo_colors::set_override(true);
    }

    #[test]
    fn format_warning_tag_does_not_exist() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_warning("tag error: tag 'v1' does not exist");
        assert!(
            result.contains("Deployment doesn't have tag 'v1'"),
            "expected tag-does-not-exist message, got: {result}"
        );

        owo_colors::set_override(true);
    }

    #[test]
    fn format_warning_fallback() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_warning("some warning");
        assert!(
            result.contains("some warning"),
            "fallback should preserve original message, got: {result}"
        );
        assert!(result.starts_with(emoji::WARNING));

        owo_colors::set_override(true);
    }

    #[test]
    fn format_warning_styled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let result = format_warning("some warning");
        assert!(result.contains("\x1b["), "styled output should contain ANSI codes");
        assert!(result.contains("some warning"));
    }

    // -- format_error tests --

    #[test]
    fn format_error_capitalizes_message() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_error("cli error: something failed");
        assert!(
            result.contains("Something failed"),
            "should capitalize first letter, got: {result}"
        );
        assert!(result.starts_with(emoji::CROSS), "should start with cross emoji");

        owo_colors::set_override(true);
    }

    #[test]
    fn format_error_no_separator() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_error("something failed");
        assert!(
            result.contains("Something failed"),
            "should capitalize even without separator, got: {result}"
        );

        owo_colors::set_override(true);
    }

    #[test]
    fn format_error_styled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let result = format_error("cli error: something failed");
        assert!(result.contains("\x1b["), "styled output should contain ANSI codes");
        assert!(result.contains("Something failed"));
    }

    // -- format_success tests --

    #[test]
    fn format_success_plain() {
        let _lock = env_lock();
        owo_colors::set_override(false);
        color::color_enabled(true);

        let result = format_success("done");
        assert_eq!(result, format!("{} done", emoji::CHECK));

        owo_colors::set_override(true);
    }

    #[test]
    fn format_success_styled() {
        let _lock = env_lock();
        let _no_color = EnvVarGuard::unset("NO_COLOR");
        let _term = EnvVarGuard::set("TERM", "xterm-256color");
        owo_colors::set_override(true);
        color::color_enabled(false);

        let result = format_success("done");
        assert!(result.contains("\x1b["), "styled output should contain ANSI codes");
        assert!(result.contains("done"));
    }

    // -- helper function tests --

    #[test]
    fn extract_message_with_separator() {
        assert_eq!(extract_message("tag error: tag 'v1' already exists"), "tag 'v1' already exists");
    }

    #[test]
    fn extract_message_without_separator() {
        assert_eq!(extract_message("some warning"), "some warning");
    }

    #[test]
    fn capitalize_first_basic() {
        assert_eq!(capitalize_first("something failed"), "Something failed");
    }

    #[test]
    fn capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn capitalize_first_already_upper() {
        assert_eq!(capitalize_first("Already upper"), "Already upper");
    }
}
