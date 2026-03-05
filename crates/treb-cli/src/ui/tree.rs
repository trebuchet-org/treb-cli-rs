//! Tree rendering with box-drawing prefixes for hierarchical data display.

/// A node in a tree structure that can be rendered with box-drawing characters.
#[derive(Debug, Clone)]
pub struct TreeNode {
    label: String,
    children: Vec<TreeNode>,
}

impl TreeNode {
    /// Create a new tree node with the given label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
        }
    }

    /// Add a child node and return self for chaining.
    pub fn child(mut self, node: TreeNode) -> Self {
        self.children.push(node);
        self
    }

    /// Render the tree into a plain-text string with box-drawing prefixes.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(self.label.clone());
        self.render_children(&mut lines, "");
        lines.join("\n")
    }

    fn render_children(&self, lines: &mut Vec<String>, prefix: &str) {
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
            child.render_children(lines, &child_prefix);
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
}
