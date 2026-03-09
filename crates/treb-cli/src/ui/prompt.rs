//! Shared confirmation and input prompt utilities.

use std::io::{self, BufRead, Write};

use console::Term;

use super::interactive::is_non_interactive;

/// Show a yes/no confirmation prompt.
///
/// In non-interactive environments (CI, pipes, `TREB_NON_INTERACTIVE=true`)
/// the `default` value is returned immediately without prompting.
pub fn confirm(message: &str, default: bool) -> bool {
    if is_non_interactive(false) {
        return default;
    }

    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
        .with_prompt(message)
        .default(default)
        .interact_on(&Term::stdout())
        .unwrap_or(default)
}

/// Show an exact raw yes/no prompt and read a single response from `input`.
pub fn confirm_raw<W: Write, R: BufRead>(
    output: &mut W,
    input: &mut R,
    prompt: &str,
) -> io::Result<bool> {
    write!(output, "{prompt}")?;
    output.flush()?;

    let mut answer = String::new();
    input.read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();

    Ok(matches!(answer.as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn confirm_returns_default_in_non_tty() {
        // Only meaningful when stdout is not a TTY.
        if console::Term::stdout().is_term() {
            return; // skip in interactive sessions
        }
        assert!(confirm("Proceed?", true));
        assert!(!confirm("Proceed?", false));
    }

    #[test]
    fn confirm_raw_writes_prompt_and_accepts_yes() {
        let mut output = Vec::new();
        let mut input = Cursor::new(b"yes\n");

        let confirmed = confirm_raw(&mut output, &mut input, "Proceed? [y/N]: ").unwrap();

        assert!(confirmed);
        assert_eq!(String::from_utf8(output).unwrap(), "Proceed? [y/N]: ");
    }

    #[test]
    fn confirm_raw_rejects_non_yes_answers() {
        let mut output = Vec::new();
        let mut input = Cursor::new(b"n\n");

        let confirmed = confirm_raw(&mut output, &mut input, "Proceed? [y/N]: ").unwrap();

        assert!(!confirmed);
    }
}
