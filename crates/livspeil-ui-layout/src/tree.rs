// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Taffy (flexbox, grid, block) over a retained node tree.
//!
//! The owner (the retained element tree in `livspeil-ui-core`) creates one
//! [`LayoutNode`] per element and pushes style, measure and children changes.
//! Setters compare before writing, so unchanged nodes stay clean and Taffy's
//! per-node cache skips them: a change only relayouts the dirty node, its
//! ancestors, and siblings whose constraints actually change.
//!
//! Pixel rounding is done here rather than by Taffy: Taffy 0.16's rounding
//! pass walks each containing block's list of hoisted out-of-flow boxes,
//! and that list goes stale when a positioned box loses its last child (it
//! is then laid out as a leaf, which never rewrites the list) or is hidden.
//! Removing such a child made the next layout panic with "invalid SlotMap
//! key"; see `docs/known-issues.md`. Our pass walks the real children and
//! only uses the hoisted lists of live ancestors to find a box's containing
//! block, so stale entries are never dereferenced.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use taffy::{AvailableSpace, NodeId, TaffyTree, compute_leaf_layout};

use crate::geometry::{Rect, Size};

/// Taffy's style (flexbox, grid, block, sizing, spacing).
pub type Style = taffy::Style;

/// A node in the [`LayoutTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayoutNode(NodeId);

/// Shaped text: one box of its natural size until it is given less width,
/// then as tall as it is when wrapped to that width.
#[derive(Clone)]
pub struct TextBlock {
    natural: Size,
    key: u64,
    height_at: Rc<dyn Fn(f32) -> f32>,
}

impl TextBlock {
    /// `natural` is the unwrapped size; `height_at` gives the height wrapped
    /// to a narrower width. `key` identifies the content and font: blocks
    /// with equal keys and natural sizes are taken to wrap alike.
    #[must_use]
    pub fn new(natural: Size, key: u64, height_at: impl Fn(f32) -> f32 + 'static) -> Self {
        TextBlock {
            natural,
            key,
            height_at: Rc::new(height_at),
        }
    }

    /// The unwrapped size.
    #[must_use]
    pub fn natural(&self) -> Size {
        self.natural
    }
}

impl PartialEq for TextBlock {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.natural == other.natural
    }
}

impl fmt::Debug for TextBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextBlock")
            .field("natural", &self.natural)
            .finish_non_exhaustive()
    }
}

/// How a leaf measures its content.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Measure {
    /// Size from style only.
    #[default]
    None,
    /// A fixed intrinsic size.
    Fixed(Size),
    /// Shaped text. Its intrinsic width is always the natural one (it never
    /// makes a flex item shrinkable); when the layout settles on a narrower
    /// width, the height is that of the wrapped text.
    Text(TextBlock),
    /// A text size estimate for when no shaper is available (the UI core
    /// uses it for text without an installed text measurer, e.g. headless
    /// tests): `chars` glyphs of average advance `0.55 × font_size`, lines
    /// of `line_height`, wrapped to the available width.
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
    pub fn measure(
        &self,
        known: taffy::Size<Option<f32>>,
        available: taffy::Size<AvailableSpace>,
    ) -> Size {
        let (w, h) = match self {
            Measure::None => (0.0, 0.0),
            Measure::Fixed(s) => (s.width, s.height),
            Measure::Text(t) => {
                // Half a pixel of rounding slack, as the painter allows.
                let height = match known.width {
                    Some(w) if w > 0.0 && w + 0.5 < t.natural.width => (t.height_at)(w),
                    _ => t.natural.height,
                };
                (t.natural.width, height)
            }
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
                let lines = if width > 0.0 {
                    (full / width).ceil().max(1.0)
                } else {
                    1.0
                };
                (width, lines * line_height)
            }
        };
        Size::new(known.width.unwrap_or(w), known.height.unwrap_or(h))
    }
}

/// A Taffy error (including an unknown or removed node).
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
    /// Rounded border boxes relative to the parent, keyed by every live node
    /// (so it doubles as the liveness check that keeps stale ids from
    /// reaching Taffy, which panics on them).
    rects: HashMap<NodeId, Rect>,
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
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        LayoutTree {
            taffy,
            rects: HashMap::new(),
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

    /// The Taffy id of a live node.
    fn id(&self, node: LayoutNode) -> Result<NodeId, LayoutError> {
        if self.rects.contains_key(&node.0) {
            Ok(node.0)
        } else {
            Err(LayoutError(taffy::TaffyError::InvalidInputNode(node.0)))
        }
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
        self.rects.insert(id, Rect::ZERO);
        Ok(LayoutNode(id))
    }

    /// Remove a node (children are detached, not removed).
    ///
    /// # Errors
    /// Taffy errors (unknown node).
    pub fn remove(&mut self, node: LayoutNode) -> Result<(), LayoutError> {
        let id = self.id(node)?;
        let parent = self.taffy.parent(id);
        self.taffy.remove(id)?;
        if let Some(p) = parent {
            self.dirty_ancestors(p)?;
        }
        self.rects.remove(&id);
        Ok(())
    }

    /// Set a node's style; returns whether it changed (and so dirtied the node).
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_style(&mut self, node: LayoutNode, style: &Style) -> Result<bool, LayoutError> {
        let id = self.id(node)?;
        if self.taffy.style(id)? == style {
            return Ok(false);
        }
        self.taffy.set_style(id, style.clone())?;
        self.dirty_ancestors(id)?;
        self.stats.style_writes += 1;
        Ok(true)
    }

    /// Set a node's measure; returns whether it changed.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_measure(&mut self, node: LayoutNode, measure: Measure) -> Result<bool, LayoutError> {
        let id = self.id(node)?;
        let current = self.taffy.get_node_context(id).cloned().unwrap_or_default();
        if current == measure {
            return Ok(false);
        }
        let ctx = (measure != Measure::None).then_some(measure);
        self.taffy.set_node_context(id, ctx)?;
        self.dirty_ancestors(id)?;
        Ok(true)
    }

    /// Set a node's children; returns whether they changed.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_children(
        &mut self,
        node: LayoutNode,
        children: &[LayoutNode],
    ) -> Result<bool, LayoutError> {
        let id = self.id(node)?;
        let ids = children
            .iter()
            .map(|c| self.id(*c))
            .collect::<Result<Vec<NodeId>, _>>()?;
        if ids.contains(&id) || ids.iter().any(|c| self.is_ancestor(*c, id)) {
            return Err(LayoutError(taffy::TaffyError::InvalidInputNode(id)));
        }
        let mut seen = std::collections::HashSet::with_capacity(ids.len());
        if !ids.iter().all(|c| seen.insert(*c)) {
            return Err(LayoutError(taffy::TaffyError::InvalidInputNode(id)));
        }
        if self.taffy.children(id)? == ids {
            return Ok(false);
        }
        let old_parents: Vec<NodeId> = ids
            .iter()
            .filter_map(|c| self.taffy.parent(*c))
            .filter(|p| *p != id)
            .collect();
        self.taffy.set_children(id, &ids)?;
        for p in old_parents {
            self.dirty_ancestors(p)?;
        }
        self.dirty_ancestors(id)?;
        Ok(true)
    }

    /// Whether the node needs layout.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn is_dirty(&self, node: LayoutNode) -> Result<bool, LayoutError> {
        Ok(self.taffy.dirty(self.id(node)?)?)
    }

    /// The node's parent, if attached.
    ///
    /// # Errors
    /// Unknown node.
    pub fn parent(&self, node: LayoutNode) -> Result<Option<LayoutNode>, LayoutError> {
        Ok(self.taffy.parent(self.id(node)?).map(LayoutNode))
    }

    /// The node's children.
    ///
    /// # Errors
    /// Unknown node.
    pub fn children(&self, node: LayoutNode) -> Result<Vec<LayoutNode>, LayoutError> {
        Ok(self
            .taffy
            .children(self.id(node)?)?
            .into_iter()
            .map(LayoutNode)
            .collect())
    }

    /// Clear the cache of `node` and every ancestor.
    ///
    /// Taffy's `mark_dirty` stops at the first node whose cache is already
    /// empty, assuming its ancestors are dirty too. Nodes under a
    /// `display: none` box break that: hidden layout clears their cache and
    /// never refills it, while the hidden box itself is cached, so a change
    /// under it was never laid out (stale rects for moved or new nodes).
    fn dirty_ancestors(&mut self, node: NodeId) -> Result<(), LayoutError> {
        let mut cur = Some(node);
        while let Some(n) = cur {
            self.taffy.mark_dirty(n)?;
            cur = self.taffy.parent(n);
        }
        Ok(())
    }

    /// Whether `ancestor` is `node` or one of its ancestors.
    fn is_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n == ancestor {
                return true;
            }
            cur = self.taffy.parent(n);
        }
        false
    }

    /// Lay out the tree under `root` in `available` space. Clean subtrees
    /// whose constraints did not change are served from Taffy's cache.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn compute(&mut self, root: LayoutNode, available: Size) -> Result<(), LayoutError> {
        let root = LayoutNode(self.id(root)?);
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
                compute_leaf_layout(
                    inputs,
                    style,
                    |_, _| 0.0,
                    |known, avail| {
                        let Some(measure) = measure else {
                            return taffy::Size::ZERO;
                        };
                        counter.set(counter.get() + 1);
                        let s = measure.measure(known, avail);
                        taffy::Size {
                            width: s.width,
                            height: s.height,
                        }
                    },
                )
            },
        )?;
        self.round(root.0);
        Ok(())
    }

    /// Pixel-snap the subtree under `root`, like Taffy's own rounding (edges
    /// are rounded in absolute coordinates, so adjacent boxes stay flush),
    /// but walking real children instead of hoisted lists.
    fn round(&mut self, root: NodeId) {
        // (node, unrounded absolute origin) for the ancestor chain.
        let mut chain: Vec<(NodeId, f32, f32)> = Vec::new();
        // (node, depth of its parent in `chain`, or `None` for the root).
        let mut stack: Vec<(NodeId, Option<usize>)> = vec![(root, None)];
        while let Some((node, parent_depth)) = stack.pop() {
            chain.truncate(parent_depth.map_or(0, |d| d + 1));
            let (px, py) = chain.last().map_or((0.0, 0.0), |&(_, x, y)| (x, y));
            // An out-of-flow box is positioned relative to its containing
            // block: the nearest ancestor that hoisted it (the compute root
            // claims whatever no positioned ancestor did).
            let style = self.taffy.style(node).ok();
            let out_of_flow = style
                .is_some_and(|s| s.position.is_out_of_flow() && s.display != taffy::Display::None);
            let (bx, by) = if out_of_flow && !chain.is_empty() {
                chain
                    .iter()
                    .rev()
                    .find(|(a, _, _)| {
                        self.taffy
                            .hoisted_children(*a)
                            .is_ok_and(|h| h.contains(&node))
                    })
                    .map_or((px, py), |&(_, x, y)| (x, y))
            } else {
                (px, py)
            };
            let l = self.taffy.unrounded_layout(node);
            let x = bx + l.location.x;
            let y = by + l.location.y;
            let rect = Rect::new(
                round(x) - round(px),
                round(y) - round(py),
                round(x + l.size.width) - round(x),
                round(y + l.size.height) - round(y),
            );
            self.rects.insert(node, rect);
            let depth = chain.len();
            chain.push((node, x, y));
            if let Ok(children) = self.taffy.children(node) {
                stack.extend(children.into_iter().rev().map(|c| (c, Some(depth))));
            }
        }
    }

    /// The node's border box relative to its parent's border box.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn rect(&self, node: LayoutNode) -> Result<Rect, LayoutError> {
        self.rects
            .get(&node.0)
            .copied()
            .ok_or(LayoutError(taffy::TaffyError::InvalidInputNode(node.0)))
    }
}

/// Round half up, as Taffy does.
fn round(v: f32) -> f32 {
    (v + 0.5).floor()
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
