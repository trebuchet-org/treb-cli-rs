//! Compact verification badge formatters.
//!
//! Renders per-verifier status (etherscan, sourcify, blockscout) in a compact
//! `e[V] s[-] b[-]` format for consistent display across CLI commands.

use std::collections::HashMap;

use owo_colors::{OwoColorize, Style};
use treb_core::types::VerifierStatus;

use super::color;

/// The three verifiers in display order.
const VERIFIER_ORDER: [(&str, &str); 3] =
    [("etherscan", "e"), ("sourcify", "s"), ("blockscout", "b")];

/// Returns a compact verification badge string for the given verifiers map.
///
/// - Empty map → `"UNVERIFIED"`
/// - Non-empty map → `"e[V] s[X] b[-]"` where V=verified, X=failed, -=missing/unknown
#[allow(dead_code)]
pub fn verification_badge(verifiers: &HashMap<String, VerifierStatus>) -> String {
    if verifiers.is_empty() {
        return "UNVERIFIED".to_string();
    }

    VERIFIER_ORDER
        .iter()
        .map(|(key, abbrev)| {
            let symbol = match verifiers.get(*key) {
                Some(vs) => status_symbol(&vs.status),
                None => "-",
            };
            format!("{}[{}]", abbrev, symbol)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns a styled verification badge with ANSI color codes around each segment.
///
/// Each badge segment is colored according to its status:
/// - Verified → [`color::VERIFIED`]
/// - Failed → [`color::FAILED`]
/// - Missing/unknown → [`color::UNVERIFIED`]
#[allow(dead_code)]
pub fn verification_badge_styled(verifiers: &HashMap<String, VerifierStatus>) -> String {
    if verifiers.is_empty() {
        return format!("{}", "UNVERIFIED".style(color::UNVERIFIED));
    }

    VERIFIER_ORDER
        .iter()
        .map(|(key, abbrev)| {
            let (symbol, style) = match verifiers.get(*key) {
                Some(vs) => (status_symbol(&vs.status), status_style(&vs.status)),
                None => ("-", color::UNVERIFIED),
            };
            let badge = format!("{}[{}]", abbrev, symbol);
            format!("{}", badge.style(style))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Maps a status string to its single-character symbol.
fn status_symbol(status: &str) -> &'static str {
    match status.to_uppercase().as_str() {
        "VERIFIED" => "V",
        "FAILED" => "X",
        _ => "-",
    }
}

/// Maps a status string to its color style.
fn status_style(status: &str) -> Style {
    match status.to_uppercase().as_str() {
        "VERIFIED" => color::VERIFIED,
        "FAILED" => color::FAILED,
        _ => color::UNVERIFIED,
    }
}

/// Returns `Some("[fork]")` if the namespace indicates a fork deployment,
/// or `None` otherwise.
///
/// A namespace is considered a fork when it starts with `"fork/"`.
#[allow(dead_code)]
pub fn fork_badge(namespace: &str) -> Option<String> {
    if namespace.starts_with("fork/") { Some("[fork]".to_string()) } else { None }
}

/// Returns a styled `[fork]` badge with yellow bold ANSI codes when the
/// namespace indicates a fork deployment, or `None` otherwise.
#[allow(dead_code)]
pub fn fork_badge_styled(namespace: &str) -> Option<String> {
    if namespace.starts_with("fork/") {
        Some(format!("{}", "[fork]".style(color::FORK_BADGE)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_verifier(status: &str) -> VerifierStatus {
        VerifierStatus { status: status.to_string(), url: String::new(), reason: String::new() }
    }

    #[test]
    fn empty_verifiers_returns_unverified() {
        let verifiers = HashMap::new();
        assert_eq!(verification_badge(&verifiers), "UNVERIFIED");
    }

    #[test]
    fn mixed_statuses_render_correctly() {
        let mut verifiers = HashMap::new();
        verifiers.insert("etherscan".to_string(), make_verifier("VERIFIED"));
        verifiers.insert("sourcify".to_string(), make_verifier("FAILED"));
        // blockscout intentionally missing

        assert_eq!(verification_badge(&verifiers), "e[V] s[X] b[-]");
    }

    #[test]
    fn order_is_always_e_s_b() {
        // Insert in reverse order to prove HashMap order doesn't matter
        let mut verifiers = HashMap::new();
        verifiers.insert("blockscout".to_string(), make_verifier("VERIFIED"));
        verifiers.insert("sourcify".to_string(), make_verifier("VERIFIED"));
        verifiers.insert("etherscan".to_string(), make_verifier("VERIFIED"));

        let badge = verification_badge(&verifiers);
        let parts: Vec<&str> = badge.split(' ').collect();
        assert!(parts[0].starts_with("e["));
        assert!(parts[1].starts_with("s["));
        assert!(parts[2].starts_with("b["));
    }

    #[test]
    fn styled_badge_contains_ansi_when_color_enabled() {
        owo_colors::set_override(true);

        let mut verifiers = HashMap::new();
        verifiers.insert("etherscan".to_string(), make_verifier("VERIFIED"));

        let styled = verification_badge_styled(&verifiers);
        assert!(
            styled.contains('\x1b'),
            "verification_badge_styled() must contain ANSI escape codes, got: {styled:?}"
        );
    }

    #[test]
    fn styled_empty_verifiers_contains_ansi() {
        owo_colors::set_override(true);

        let verifiers = HashMap::new();
        let styled = verification_badge_styled(&verifiers);
        assert!(
            styled.contains('\x1b'),
            "styled UNVERIFIED must contain ANSI codes, got: {styled:?}"
        );
    }

    #[test]
    fn all_verified_renders_all_v() {
        let mut verifiers = HashMap::new();
        verifiers.insert("etherscan".to_string(), make_verifier("VERIFIED"));
        verifiers.insert("sourcify".to_string(), make_verifier("VERIFIED"));
        verifiers.insert("blockscout".to_string(), make_verifier("VERIFIED"));

        assert_eq!(verification_badge(&verifiers), "e[V] s[V] b[V]");
    }

    #[test]
    fn unknown_status_renders_as_dash() {
        let mut verifiers = HashMap::new();
        verifiers.insert("etherscan".to_string(), make_verifier("PARTIAL"));

        let badge = verification_badge(&verifiers);
        assert!(badge.starts_with("e[-]"), "Unknown status should map to -, got: {badge}");
    }

    // -----------------------------------------------------------------------
    // Fork badge tests
    // -----------------------------------------------------------------------

    #[test]
    fn fork_badge_returns_some_for_fork_namespace() {
        assert_eq!(fork_badge("fork/42220"), Some("[fork]".to_string()));
    }

    #[test]
    fn fork_badge_returns_none_for_non_fork_namespace() {
        assert_eq!(fork_badge("mainnet"), None);
    }

    #[test]
    fn fork_badge_returns_none_for_partial_prefix() {
        assert_eq!(fork_badge("forked/42220"), None);
    }

    #[test]
    fn fork_badge_styled_contains_ansi_when_color_enabled() {
        owo_colors::set_override(true);

        let styled = fork_badge_styled("fork/42220");
        assert!(styled.is_some(), "fork_badge_styled must return Some for fork namespace");
        let styled = styled.unwrap();
        assert!(
            styled.contains('\x1b'),
            "fork_badge_styled() must contain ANSI escape codes, got: {styled:?}"
        );
        assert!(
            styled.contains("[fork]"),
            "styled badge must contain [fork] text, got: {styled:?}"
        );
    }

    #[test]
    fn fork_badge_styled_returns_none_for_non_fork() {
        assert_eq!(fork_badge_styled("mainnet"), None);
    }
}
