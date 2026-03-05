//! Tree rendering with box-drawing prefixes for hierarchical data display.

use owo_colors::{OwoColorize, Style};

/// A node in a tree structure that can be rendered with box-drawing characters.
#[derive(Debug, Clone)]
pub struct TreeNode {
    label: String,
    style: Option<Style>,
    children: Vec<TreeNode>,
}

impl TreeNode {
    /// Create a new tree node with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: None,
            children: Vec::new(),
        }
    }

    /// Set the ANSI style for this node's label and return self for chaining.
    pub fn with_style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Add a child node and return self for chaining.
    pub fn child(mut self, node: TreeNode) -> Self {
        self.children.push(node);
        self
    }

    /// Render the tree into a plain-text string with box-drawing prefixes.
    /// No ANSI escape codes are included in the output.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(self.label.clone());
        self.render_children_plain(&mut lines, "");
        lines.join("\n")
    }

    /// Render the tree with ANSI-styled labels. Tree prefix characters remain
    /// uncolored; only label text receives the node's style (if set).
    pub fn render_styled(&self) -> String {
        let mut lines = Vec::new();
        lines.push(self.styled_label());
        self.render_children_styled(&mut lines, "");
        lines.join("\n")
    }

    fn styled_label(&self) -> String {
        match self.style {
            Some(style) => format!("{}", self.label.style(style)),
            None => self.label.clone(),
        }
    }

    fn render_children_plain(&self, lines: &mut Vec<String>, prefix: &str) {
        let count = self.children.len();
        for (i, child) in self.children.iter().enumerate() {
            let is_last = i == count - 1;
            let connector = if is_last { "\\-- " } else { "|-- " };
            lines.push(format!("{}{}{}", prefix, connector, child.label));

            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}|   ", prefix)
            };
            child.render_children_plain(lines, &child_prefix);
        }
    }

    fn render_children_styled(&self, lines: &mut Vec<String>, prefix: &str) {
        let count = self.children.len();
        for (i, child) in self.children.iter().enumerate() {
            let is_last = i == count - 1;
            let connector = if is_last { "\\-- " } else { "|-- " };
            lines.push(format!("{}{}{}", prefix, connector, child.styled_label()));

            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}|   ", prefix)
            };
            child.render_children_styled(lines, &child_prefix);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_node_renders_label_only() {
        let tree = TreeNode::new("root");
        assert_eq!(tree.render(), "root");
    }

    #[test]
    fn two_children_render_with_prefixes() {
        let tree = TreeNode::new("root")
            .child(TreeNode::new("a"))
            .child(TreeNode::new("b"));
        let rendered = tree.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "root");
        assert_eq!(lines[1], "|-- a");
        assert_eq!(lines[2], "\\-- b");
    }

    #[test]
    fn three_level_nesting_has_correct_continuation_prefixes() {
        let tree = TreeNode::new("root")
            .child(
                TreeNode::new("mid1")
                    .child(TreeNode::new("leaf1"))
                    .child(TreeNode::new("leaf2")),
            )
            .child(TreeNode::new("mid2").child(TreeNode::new("leaf3")));
        let rendered = tree.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0], "root");
        assert_eq!(lines[1], "|-- mid1");
        assert_eq!(lines[2], "|   |-- leaf1");
        assert_eq!(lines[3], "|   \\-- leaf2");
        assert_eq!(lines[4], "\\-- mid2");
        assert_eq!(lines[5], "    \\-- leaf3");
    }

    #[test]
    fn empty_children_renders_just_label() {
        let tree = TreeNode::new("standalone");
        let rendered = tree.render();
        assert_eq!(rendered.lines().count(), 1);
        assert_eq!(rendered, "standalone");
    }

    #[test]
    fn render_plain_never_contains_ansi_escapes() {
        let tree = TreeNode::new("root")
            .with_style(Style::new().red().bold())
            .child(TreeNode::new("child").with_style(Style::new().green()));
        let rendered = tree.render();
        assert!(
            !rendered.contains('\x1b'),
            "render() must not contain ANSI escape codes, got: {rendered:?}"
        );
    }

    #[test]
    fn render_styled_contains_ansi_around_labels() {
        owo_colors::set_override(true);

        let tree = TreeNode::new("root")
            .with_style(Style::new().red())
            .child(TreeNode::new("child").with_style(Style::new().green()));
        let styled = tree.render_styled();

        assert!(
            styled.contains('\x1b'),
            "render_styled() must contain ANSI escape codes when color is enabled"
        );

        // Verify prefix characters are NOT wrapped in ANSI
        let lines: Vec<&str> = styled.lines().collect();
        // The child line should start with the plain prefix, not an escape
        assert!(
            lines[1].starts_with("\\-- "),
            "Tree prefix must not be colored, got: {:?}",
            lines[1]
        );
    }

    #[test]
    fn styled_vs_plain_differ_only_in_ansi_codes() {
        owo_colors::set_override(true);

        let style = Style::new().cyan().bold();
        let tree = TreeNode::new("root")
            .with_style(style)
            .child(TreeNode::new("a").with_style(style))
            .child(TreeNode::new("b").with_style(style));

        let plain = tree.render();
        let styled = tree.render_styled();

        // Same number of lines
        assert_eq!(plain.lines().count(), styled.lines().count());

        // Stripping ANSI from styled should recover plain text
        let ansi_re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
        let stripped = ansi_re.replace_all(&styled, "");
        assert_eq!(plain, stripped.as_ref());
    }

    #[test]
    fn render_styled_without_style_matches_plain() {
        let tree = TreeNode::new("root")
            .child(TreeNode::new("a"))
            .child(TreeNode::new("b"));
        assert_eq!(tree.render(), tree.render_styled());
    }
}
