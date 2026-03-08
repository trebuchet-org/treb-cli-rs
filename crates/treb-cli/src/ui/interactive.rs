//! Centralized non-interactive mode detection.
//!
//! A command is considered non-interactive when **any** of the following hold:
//!
//! 1. The `--non-interactive` CLI flag was passed (`cli_flag` parameter).
//! 2. The `TREB_NON_INTERACTIVE` environment variable is set to `"true"` (case-insensitive).
//! 3. The `CI` environment variable is set to `"true"` (case-insensitive).
//! 4. Standard input is not a terminal (piped / redirected).

use std::io::IsTerminal;

/// Returns `true` when interactive prompts should be suppressed.
///
/// `cli_flag` is the value of the `--non-interactive` CLI argument (if the
/// command exposes one).  Pass `false` when the calling command has no such
/// flag.
pub fn is_non_interactive(cli_flag: bool) -> bool {
    if cli_flag {
        return true;
    }

    if let Ok(val) = std::env::var("TREB_NON_INTERACTIVE") {
        if val.eq_ignore_ascii_case("true") {
            return true;
        }
    }

    if let Ok(val) = std::env::var("CI") {
        if val.eq_ignore_ascii_case("true") {
            return true;
        }
    }

    !std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flag_forces_non_interactive() {
        assert!(is_non_interactive(true));
    }

    #[test]
    fn non_tty_is_non_interactive() {
        // In CI / piped test runners stdin is not a TTY, so this should be true.
        // In an interactive terminal this test is still valid because the
        // function falls through to the TTY check.
        let result = is_non_interactive(false);
        // We can't assert a specific value here because it depends on the
        // runner environment, but we verify it doesn't panic.
        let _ = result;
    }
}
