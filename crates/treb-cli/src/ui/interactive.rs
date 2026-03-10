//! Centralized non-interactive mode detection.
//!
//! A command is considered non-interactive when **any** of the following hold:
//!
//! 1. The `--non-interactive` CLI flag was passed (`cli_flag` parameter).
//! 2. The `TREB_NON_INTERACTIVE` environment variable is set to `"1"` or `"true"`
//!    (case-insensitive).
//! 3. The `CI` environment variable is set to `"true"` (case-insensitive).
//! 4. Standard input is not a terminal (piped / redirected).
//! 5. Standard output is not a terminal (redirected).

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

    if env_requests_non_interactive() {
        return true;
    }

    is_non_interactive_for_terminals(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )
}

fn env_requests_non_interactive() -> bool {
    matches!(
        std::env::var("TREB_NON_INTERACTIVE"),
        Ok(val) if env_var_requests_non_interactive(&val)
    ) || matches!(std::env::var("CI"), Ok(val) if val.eq_ignore_ascii_case("true"))
}

fn env_var_requests_non_interactive(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

fn is_non_interactive_for_terminals(stdin_is_terminal: bool, stdout_is_terminal: bool) -> bool {
    !stdin_is_terminal || !stdout_is_terminal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_flag_forces_non_interactive() {
        assert!(is_non_interactive(true));
    }

    #[test]
    fn treb_non_interactive_env_accepts_one() {
        assert!(env_var_requests_non_interactive("1"));
    }

    #[test]
    fn treb_non_interactive_env_accepts_true_case_insensitively() {
        assert!(env_var_requests_non_interactive("true"));
        assert!(env_var_requests_non_interactive("TRUE"));
    }

    #[test]
    fn treb_non_interactive_env_rejects_falsey_values() {
        assert!(!env_var_requests_non_interactive("0"));
        assert!(!env_var_requests_non_interactive("false"));
        assert!(!env_var_requests_non_interactive("FALSE"));
    }

    #[test]
    fn both_ttys_allow_interaction() {
        assert!(!is_non_interactive_for_terminals(true, true));
    }

    #[test]
    fn redirected_stdin_is_non_interactive() {
        assert!(is_non_interactive_for_terminals(false, true));
    }

    #[test]
    fn redirected_stdout_is_non_interactive() {
        assert!(is_non_interactive_for_terminals(true, false));
    }
}
