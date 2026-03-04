//! Color output control and palette constants.
//!
//! Respects the `NO_COLOR` environment variable (<https://no-color.org/>) and
//! the `TERM=dumb` convention. Use [`should_use_color`] to check whether color
//! output is appropriate, and [`color_enabled`] to query or apply the decision
//! to the owo-colors subsystem.

use owo_colors::Style;

// ---------------------------------------------------------------------------
// Palette constants
// ---------------------------------------------------------------------------

#[allow(dead_code)]
/// Color/style for stage/section headings.
pub const STAGE: Style = Style::new().cyan().bold();

#[allow(dead_code)]
/// Color/style for success messages.
pub const SUCCESS: Style = Style::new().green().bold();

#[allow(dead_code)]
/// Color/style for warnings.
pub const WARNING: Style = Style::new().yellow();

#[allow(dead_code)]
/// Color/style for errors.
pub const ERROR: Style = Style::new().red().bold();

#[allow(dead_code)]
/// Color/style for muted/secondary text.
pub const MUTED: Style = Style::new().dimmed();

// ---------------------------------------------------------------------------
// Color enable / disable helpers
// ---------------------------------------------------------------------------

/// Returns `true` if color output should be used based on the current environment.
///
/// Returns `false` when:
/// - The `NO_COLOR` environment variable is set to any value (per <https://no-color.org/>)
/// - The `TERM` environment variable is `dumb`
pub fn should_use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").ok().as_deref() == Some("dumb") {
        return false;
    }
    true
}

/// Returns whether color output is currently enabled.
///
/// This combines the environment check from [`should_use_color`] with any
/// runtime override applied via `--no-color`.  When `override_disabled` is
/// `true` the function unconditionally returns `false` *and* disables
/// colorization in the owo-colors subsystem for the remainder of the process.
pub fn color_enabled(override_disabled: bool) -> bool {
    let enabled = !override_disabled && should_use_color();
    if !enabled {
        owo_colors::set_override(false);
    }
    enabled
}

// ---------------------------------------------------------------------------
// Compile-time Send + Sync check for palette constants
// ---------------------------------------------------------------------------

const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}
    fn check() {
        assert_send_sync::<Style>();
    }
    let _ = check;
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_env_disables_color() {
        // Safety: single-threaded test manipulating env vars.
        // This test is not safe to run in parallel with other env-var tests,
        // but cargo test runs unit tests in a single binary where this is
        // the only writer, so it is acceptable.
        unsafe {
            std::env::set_var("NO_COLOR", "1");
        }
        let result = should_use_color();
        unsafe {
            std::env::remove_var("NO_COLOR");
        }
        assert!(!result, "should_use_color() must return false when NO_COLOR is set");
    }

    #[test]
    fn term_dumb_disables_color() {
        let old_term = std::env::var("TERM").ok();
        unsafe {
            std::env::set_var("TERM", "dumb");
        }
        let result = should_use_color();
        unsafe {
            match old_term {
                Some(v) => std::env::set_var("TERM", v),
                None => std::env::remove_var("TERM"),
            }
        }
        assert!(!result, "should_use_color() must return false when TERM=dumb");
    }
}
