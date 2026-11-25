// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Accessibility glue between the app and the platform adapter
//! (`accesskit_winit`), without a window so it is tested headlessly.
//!
//! - The adapter asks for the initial tree when assistive technology
//!   (VoiceOver: Cmd+F5) starts: [`Accessibility::initial_tree`] sends the
//!   full tree.
//! - While active, after each frame [`Accessibility::update`] sends only the
//!   nodes that changed (`livspeil-ui-a11y`'s [`A11yTree`]), or nothing.
//! - Action requests (press, focus, increment, set value…) go to
//!   [`livspeil_ui_a11y::handle_action`].
//! - The window title names the root node unless the app labels it.

use livspeil_ui_a11y::accesskit::{
    ActionRequest, Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate,
};
use livspeil_ui_a11y::{A11yError, A11yTree, TOOLKIT_NAME};
use livspeil_ui_core::App;

/// Node id of the placeholder root sent before the app has a tree.
const PLACEHOLDER: NodeId = NodeId(u64::MAX);

/// The app side of the platform accessibility adapter.
#[derive(Debug)]
pub struct Accessibility {
    tree: A11yTree,
    title: String,
    active: bool,
}

impl Accessibility {
    /// Glue for a window titled `title` (the root's accessible name).
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Accessibility {
            tree: A11yTree::new(),
            title: title.into(),
            active: false,
        }
    }

    /// Whether assistive technology asked for the tree (and did not
    /// deactivate since).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The window title.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Change the title (sent with the next update).
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
        // Resend the root with its new name.
        self.tree.reset();
    }

    /// The mirrored tree as of the last update.
    #[must_use]
    pub fn tree(&self) -> &A11yTree {
        &self.tree
    }

    /// The full tree (activation). Before the app has mounted a root, a
    /// window node named after the title.
    pub fn initial_tree(&mut self, app: &App, scale: f64) -> TreeUpdate {
        self.active = true;
        match self.tree.full_update(app, scale) {
            Some(update) => self.named(update),
            None => {
                let mut node = Node::new(Role::Window);
                node.set_label(self.title.as_str());
                TreeUpdate {
                    nodes: vec![(PLACEHOLDER, node)],
                    tree: Some(TreeInfo {
                        root: PLACEHOLDER,
                        toolkit_name: Some(TOOLKIT_NAME.to_owned()),
                        toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
                    }),
                    tree_id: TreeId::ROOT,
                    focus: PLACEHOLDER,
                }
            }
        }
    }

    /// The changes since the last update, while active; `None` when
    /// inactive or nothing changed.
    pub fn update(&mut self, app: &App, scale: f64) -> Option<TreeUpdate> {
        if !self.active {
            return None;
        }
        let update = self.tree.update(app, scale)?;
        Some(self.named(update))
    }

    /// Assistive technology went away: stop sending updates; the next
    /// activation sends the full tree again.
    pub fn deactivate(&mut self) {
        self.active = false;
        self.tree.reset();
    }

    /// Perform an action request on the app. Returns whether something
    /// handled it.
    ///
    /// # Errors
    /// See [`A11yError`].
    pub fn handle_action(app: &mut App, request: ActionRequest) -> Result<bool, A11yError> {
        livspeil_ui_a11y::handle_action(app, request)
    }

    fn named(&self, mut update: TreeUpdate) -> TreeUpdate {
        if let Some(root) = self.tree.root() {
            for (id, node) in &mut update.nodes {
                if *id == root && node.label().is_none() {
                    node.set_label(self.title.as_str());
                }
            }
        }
        update
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_before_the_root_is_mounted() {
        let app = App::new();
        let mut a = Accessibility::new("Gallery");
        assert!(!a.is_active());
        let t = a.initial_tree(&app, 2.0);
        assert!(a.is_active());
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.nodes[0].1.label(), Some("Gallery"));
        assert_eq!(t.tree.map(|i| i.root), Some(PLACEHOLDER));
        a.deactivate();
        assert!(a.update(&app, 2.0).is_none());
    }
}
