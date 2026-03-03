//! Shared confirmation and input prompt utilities.

use console::Term;

/// Show a yes/no confirmation prompt.
///
/// In non-TTY environments (e.g. CI, pipes) the `default` value is returned
/// immediately without prompting.
pub fn confirm(message: &str, default: bool) -> bool {
    let is_tty = Term::stdout().is_term();
    if !is_tty {
        return default;
    }

    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(message)
        .default(default)
        .interact_on(&Term::stdout())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_returns_default_in_non_tty() {
        // Only meaningful when stdout is not a TTY.
        if console::Term::stdout().is_term() {
            return; // skip in interactive sessions
        }
        assert!(confirm("Proceed?", true));
        assert!(!confirm("Proceed?", false));
    }
}
