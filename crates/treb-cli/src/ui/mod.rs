//! Interactive UI utilities: fuzzy selectors, confirmation prompts, and color output.

pub mod badge;
pub mod color;
pub mod prompt;
pub mod selector;
pub mod terminal;
pub mod tree;

#[cfg(test)]
mod ui_integration_tests {
    use std::collections::HashMap;

    use treb_core::types::VerifierStatus;

    use super::badge::{fork_badge, fork_badge_styled, verification_badge, verification_badge_styled};
    use super::color;
    use super::terminal::display_width;
    use super::tree::TreeNode;

    /// Build a realistic 3-level tree: namespace > chain > deployment.
    /// Exercises styled labels, verification badges, and fork badges together.
    #[test]
    fn framework_composes_three_level_tree() {
        owo_colors::set_override(true);

        // -- Build verification badge for a deployment --
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        verifiers.insert(
            "sourcify".to_string(),
            VerifierStatus { status: "FAILED".to_string(), url: String::new(), reason: String::new() },
        );
        let badge_plain = verification_badge(&verifiers);
        let badge_styled = verification_badge_styled(&verifiers);

        // -- Build fork badge --
        let fork = fork_badge("fork/42220").unwrap();
        let fork_styled = fork_badge_styled("fork/42220").unwrap();

        // -- Assemble deployment labels --
        let deploy_label = format!("MyProxy 0xabcd {badge_plain} {fork}");
        let _deploy_label_styled_text = format!("MyProxy 0xabcd {badge_styled} {fork_styled}");

        // -- Build 3-level tree --
        let tree = TreeNode::new("mainnet")
            .with_style(color::NAMESPACE)
            .child(
                TreeNode::new("ethereum")
                    .with_style(color::CHAIN)
                    .child(TreeNode::new("Singleton 0x1111"))
                    .child(TreeNode::new(deploy_label.clone())),
            )
            .child(
                TreeNode::new("optimism")
                    .with_style(color::CHAIN)
                    .child(TreeNode::new("Library 0x2222")),
            );

        // -- Plain rendering --
        let plain = tree.render();
        let plain_lines: Vec<&str> = plain.lines().collect();

        // 3-level tree: root + 2 chains + 3 deployments = 6 lines
        assert_eq!(plain_lines.len(), 6, "Expected 6 lines, got:\n{plain}");

        // Check root label
        assert_eq!(plain_lines[0], "mainnet");

        // Check prefix characters
        assert!(plain_lines[1].starts_with("|-- "), "First chain should use |-- prefix");
        assert!(plain_lines[1].contains("ethereum"), "First chain label");
        assert!(plain_lines[2].starts_with("|   |-- "), "First deployment of first chain");
        assert!(plain_lines[3].starts_with("|   \\-- "), "Last deployment of first chain");
        assert!(plain_lines[4].starts_with("\\-- "), "Last chain should use \\-- prefix");
        assert!(plain_lines[5].starts_with("    \\-- "), "Only child of last chain");

        // Check deployment label content
        assert!(plain_lines[3].contains("MyProxy"), "Deployment label should contain name");
        assert!(plain_lines[3].contains("e[V]"), "Deployment label should contain verification badge");
        assert!(plain_lines[3].contains("[fork]"), "Deployment label should contain fork badge");

        // Plain output must not contain ANSI
        assert!(!plain.contains('\x1b'), "render() must not contain ANSI codes");

        // -- Styled rendering --
        let styled = tree.render_styled();
        let styled_lines: Vec<&str> = styled.lines().collect();

        // Same number of lines
        assert_eq!(
            styled_lines.len(),
            plain_lines.len(),
            "Styled and plain must have same line count"
        );

        // Styled output should contain ANSI codes (from styled labels)
        assert!(styled.contains('\x1b'), "render_styled() must contain ANSI codes");

        // Prefix characters should not be colored
        assert!(
            styled_lines[1].starts_with("|-- "),
            "Styled prefix must not be colored: {:?}",
            styled_lines[1]
        );
        assert!(
            styled_lines[4].starts_with("\\-- "),
            "Styled prefix must not be colored: {:?}",
            styled_lines[4]
        );
    }

    /// Verify display_width produces consistent widths for plain and styled
    /// versions of the same label text.
    #[test]
    fn display_width_consistent_plain_vs_styled() {
        owo_colors::set_override(true);

        // Build some label fragments
        let mut verifiers = HashMap::new();
        verifiers.insert(
            "etherscan".to_string(),
            VerifierStatus { status: "VERIFIED".to_string(), url: String::new(), reason: String::new() },
        );
        let badge_plain = verification_badge(&verifiers);
        let badge_styled = verification_badge_styled(&verifiers);

        let fork_plain = fork_badge("fork/1234").unwrap();
        let fork_styled_val = fork_badge_styled("fork/1234").unwrap();

        // Plain and styled versions of the same logical content
        let plain_label = format!("MyContract 0xdead {badge_plain} {fork_plain}");
        let styled_label = format!("MyContract 0xdead {badge_styled} {fork_styled_val}");

        let plain_w = display_width(&plain_label);
        let styled_w = display_width(&styled_label);

        assert_eq!(
            plain_w, styled_w,
            "display_width must be equal for plain ({plain_w}) and styled ({styled_w}) versions"
        );
        assert!(plain_w > 0, "display_width must be positive");
    }

    /// Non-fork namespace should not produce a fork badge.
    #[test]
    fn non_fork_namespace_excluded_from_tree() {
        let tree = TreeNode::new("production")
            .child(TreeNode::new("ethereum").child(TreeNode::new("Contract 0x1234")));

        let rendered = tree.render();
        assert!(!rendered.contains("[fork]"), "Non-fork tree should not contain [fork]");
        assert_eq!(fork_badge("production"), None);
    }
}
