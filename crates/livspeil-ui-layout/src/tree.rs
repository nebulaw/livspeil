// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Taffy (flexbox, grid, block) over a retained node tree.
//!
//! The owner (the retained element tree in `livspeil-ui-core`) creates one
//! [`LayoutNode`] per element and pushes style, measure and children changes.
//! Setters compare before writing, so unchanged nodes stay clean and Taffy's
//! per-node cache skips them: a change only relayouts the dirty node, its
//! ancestors, and siblings whose constraints actually change.

use std::cell::Cell;
use std::fmt;

use taffy::{AvailableSpace, NodeId, TaffyTree, compute_leaf_layout};

use crate::geometry::{Rect, Size};

/// Taffy's style (flexbox, grid, block, sizing, spacing).
pub type Style = taffy::Style;

/// A node in the [`LayoutTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutNode(NodeId);

/// How a leaf measures its content.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Measure {
    /// Size from style only.
    #[default]
    None,
    /// A fixed intrinsic size.
    Fixed(Size),
    /// Placeholder for text until `livspeil-ui-text` lands: `chars` glyphs
    /// of average advance `0.55 × font_size`, one line of `line_height`,
    /// wrapped to the available width.
    TextPlaceholder {
        /// Number of characters.
        chars: u32,
        /// Font size.
        font_size: f32,
        /// Line height.
        line_height: f32,
    },
}

impl Measure {
    /// Intrinsic size under the known dimensions and available space.
    #[must_use]
    pub fn measure(&self, known: taffy::Size<Option<f32>>, available: taffy::Size<AvailableSpace>) -> Size {
        let (w, h) = match self {
            Measure::None => (0.0, 0.0),
            Measure::Fixed(s) => (s.width, s.height),
            Measure::TextPlaceholder {
                chars,
                font_size,
                line_height,
            } => {
                #[allow(clippy::cast_precision_loss)]
                let full = *chars as f32 * font_size * 0.55;
                let max = match available.width {
                    AvailableSpace::Definite(w) => w.max(0.0),
                    AvailableSpace::MinContent => font_size * 0.55 * 8.0,
                    AvailableSpace::MaxContent => f32::INFINITY,
                };
                let width = known.width.unwrap_or(full.min(max));
                let lines = if width > 0.0 { (full / width).ceil().max(1.0) } else { 1.0 };
                (width, lines * line_height)
            }
        };
        Size::new(known.width.unwrap_or(w), known.height.unwrap_or(h))
    }
}

/// A Taffy error.
#[derive(Debug)]
pub struct LayoutError(taffy::TaffyError);

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layout: {}", self.0)
    }
}

impl std::error::Error for LayoutError {}

impl From<taffy::TaffyError> for LayoutError {
    fn from(e: taffy::TaffyError) -> Self {
        LayoutError(e)
    }
}

/// Counters for tests and profiling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// Measure function calls over the tree's lifetime.
    pub measure_calls: usize,
    /// `compute` calls.
    pub computes: usize,
    /// Style writes that actually changed a style.
    pub style_writes: usize,
}

/// A retained Taffy tree.
pub struct LayoutTree {
    taffy: TaffyTree<Measure>,
    measure_calls: Cell<usize>,
    stats: LayoutStats,
}

impl fmt::Debug for LayoutTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayoutTree")
            .field("nodes", &self.taffy.total_node_count())
            .field("stats", &self.stats())
            .finish()
    }
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutTree {
    /// An empty tree.
    #[must_use]
    pub fn new() -> Self {
        LayoutTree {
            taffy: TaffyTree::new(),
            measure_calls: Cell::new(0),
            stats: LayoutStats::default(),
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.taffy.total_node_count()
    }

    /// Whether the tree has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> LayoutStats {
        LayoutStats {
            measure_calls: self.measure_calls.get(),
            ..self.stats
        }
    }

    /// Create a node.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn create(&mut self, style: Style, measure: Measure) -> Result<LayoutNode, LayoutError> {
        let id = if measure == Measure::None {
            self.taffy.new_leaf(style)?
        } else {
            self.taffy.new_leaf_with_context(style, measure)?
        };
        Ok(LayoutNode(id))
    }

    /// Remove a node (children are detached, not removed).
    ///
    /// # Errors
    /// Taffy errors (unknown node).
    pub fn remove(&mut self, node: LayoutNode) -> Result<(), LayoutError> {
        self.taffy.remove(node.0)?;
        Ok(())
    }

    /// Set a node's style; returns whether it changed (and so dirtied the node).
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_style(&mut self, node: LayoutNode, style: &Style) -> Result<bool, LayoutError> {
        if self.taffy.style(node.0)? == style {
            return Ok(false);
        }
        self.taffy.set_style(node.0, style.clone())?;
        self.stats.style_writes += 1;
        Ok(true)
    }

    /// Set a node's measure; returns whether it changed.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_measure(&mut self, node: LayoutNode, measure: Measure) -> Result<bool, LayoutError> {
        let current = self.taffy.get_node_context(node.0).cloned().unwrap_or_default();
        if current == measure {
            return Ok(false);
        }
        let ctx = (measure != Measure::None).then_some(measure);
        self.taffy.set_node_context(node.0, ctx)?;
        Ok(true)
    }

    /// Set a node's children; returns whether they changed.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_children(&mut self, node: LayoutNode, children: &[LayoutNode]) -> Result<bool, LayoutError> {
        let current = self.taffy.children(node.0)?;
        if current.len() == children.len() && current.iter().zip(children).all(|(a, b)| *a == b.0) {
            return Ok(false);
        }
        let ids: Vec<NodeId> = children.iter().map(|c| c.0).collect();
        self.taffy.set_children(node.0, &ids)?;
        Ok(true)
    }

    /// Whether the node needs layout.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn is_dirty(&self, node: LayoutNode) -> Result<bool, LayoutError> {
        Ok(self.taffy.dirty(node.0)?)
    }

    /// Lay out the tree under `root` in `available` space. Clean subtrees
    /// whose constraints did not change are served from Taffy's cache.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn compute(&mut self, root: LayoutNode, available: Size) -> Result<(), LayoutError> {
        self.stats.computes += 1;
        let counter = &self.measure_calls;
        self.taffy.compute_layout_with_measure(
            root.0,
            taffy::Size {
                width: AvailableSpace::Definite(available.width),
                height: AvailableSpace::Definite(available.height),
            },
            |inputs, _id, ctx, style| {
                let measure = ctx.map(|m| &*m);
                compute_leaf_layout(inputs, style, |_, _| 0.0, |known, avail| {
                    let Some(measure) = measure else {
                        return taffy::Size::ZERO;
                    };
                    counter.set(counter.get() + 1);
                    let s = measure.measure(known, avail);
                    taffy::Size {
                        width: s.width,
                        height: s.height,
                    }
                })
            },
        )?;
        Ok(())
    }

    /// The node's border box relative to its parent's border box.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn rect(&self, node: LayoutNode) -> Result<Rect, LayoutError> {
        let l = self.taffy.layout(node.0)?;
        Ok(Rect::new(l.location.x, l.location.y, l.size.width, l.size.height))
    }
}

/// Style helpers that keep callers off Taffy's verbose constructors.
pub mod style {
    pub use taffy::style_helpers::{fr, length, percent};
    pub use taffy::{
        AlignContent, AlignItems, AlignSelf, Dimension, Display, FlexDirection, FlexWrap,
        GridPlacement, JustifyContent, LengthPercentage, LengthPercentageAuto, Overflow, Position,
        TrackSizingFunction,
    };
}
