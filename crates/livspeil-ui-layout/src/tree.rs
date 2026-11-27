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
//! **Layout boundaries.** An absolutely positioned box with a definite size
//! (length or percentage in both directions) cannot be sized by its
//! contents, so its contents are laid out as a Taffy tree of their own: the
//! box is a leaf of its parent's tree, and a change inside it dirties and
//! lays out only the box's subtree. When such a box is placed by `left` and
//! `top` alone, those two are not given to Taffy at all: moving it changes
//! an offset that is added when rounding, and runs no layout.
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
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use taffy::{
    AvailableSpace, BoxSizing, CompactLength, Dimension, Display, LengthPercentage,
    LengthPercentageAuto, NodeId, Position, TaffyTree, compute_leaf_layout,
};

use crate::geometry::{Rect, Size, Vec2};

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
    /// Layouts of the main tree (from the node given to `compute`).
    pub computes: usize,
    /// Layouts of the contents of a layout boundary.
    pub boundary_computes: usize,
    /// Boxes moved without a layout.
    pub moves: usize,
    /// Style writes that actually changed a style.
    pub style_writes: usize,
}

/// What a [`LayoutTree::compute`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutOutcome {
    /// Whether any layout algorithm ran (moving a box runs none).
    pub ran: bool,
    /// The roots of the subtrees whose rects may have changed: the node
    /// given to `compute`, layout boundaries and moved boxes. Rects outside
    /// these subtrees are as before.
    pub changed: Vec<LayoutNode>,
}

/// One node as its owner sees it.
struct Entry {
    /// The style as given.
    style: Style,
    /// For a layout boundary with children: the root of its content tree.
    content: Option<NodeId>,
    /// `left`/`top` kept out of Taffy (see the module docs).
    offset: Vec2,
    /// Rounded border box relative to the parent.
    rect: Rect,
    /// The containing block's origin and the parent's origin in the
    /// rounding frame of the last pass, to re-round a moved box.
    base: Vec2,
    frame: Vec2,
    /// Whether it was under `display: none` in the last pass.
    hidden: bool,
}

/// A retained Taffy tree.
pub struct LayoutTree {
    taffy: TaffyTree<Measure>,
    /// Keyed by every live node (so it doubles as the liveness check that
    /// keeps stale ids from reaching Taffy, which panics on them).
    entries: HashMap<NodeId, Entry>,
    /// Content root to the node that owns it.
    owners: HashMap<NodeId, NodeId>,
    /// Boundaries with a change inside, and boxes that only moved.
    pending: HashSet<NodeId>,
    moved: HashSet<NodeId>,
    /// The space of the last main layout.
    available: Option<Size>,
    /// Whether boundaries are used (a test compares with plain Taffy).
    boundaries: bool,
    measure_calls: Cell<usize>,
    stats: LayoutStats,
}

impl fmt::Debug for LayoutTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayoutTree")
            .field("nodes", &self.entries.len())
            .field("boundaries", &self.owners.len())
            .field("stats", &self.stats())
            .finish()
    }
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the size cannot depend on the contents.
fn definite(d: Dimension) -> bool {
    d.into_raw().is_length_or_percentage()
}

fn is_length(v: LengthPercentageAuto) -> bool {
    v.into_raw().tag() == CompactLength::LENGTH_TAG
}

/// Percentage padding resolves against the containing block, which the
/// contents of a boundary do not see.
fn fixed_padding(v: LengthPercentage) -> bool {
    let v = v.into_raw();
    v.tag() == CompactLength::LENGTH_TAG
        || (v.tag() == CompactLength::PERCENT_TAG && v.value() == 0.0)
}

/// An out-of-flow box whose size is known without looking inside it.
fn sized_out_of_flow(s: &Style) -> bool {
    s.position == Position::Absolute
        && s.display != Display::None
        && definite(s.size.width)
        && definite(s.size.height)
}

fn is_boundary(s: &Style) -> bool {
    let p = &s.padding;
    sized_out_of_flow(s)
        && [p.left, p.right, p.top, p.bottom]
            .into_iter()
            .all(fixed_padding)
}

/// The style Taffy places the box with, and the offset taken out of it.
fn placement(style: &Style) -> (Style, Vec2) {
    let i = &style.inset;
    let movable = sized_out_of_flow(style)
        && is_length(i.left)
        && is_length(i.top)
        && i.right.is_auto()
        && i.bottom.is_auto();
    let mut s = style.clone();
    if !movable {
        return (s, Vec2::ZERO);
    }
    let offset = Vec2::new(i.left.into_raw().value(), i.top.into_raw().value());
    s.inset.left = LengthPercentageAuto::length(0.0);
    s.inset.top = LengthPercentageAuto::length(0.0);
    (s, offset)
}

/// The style of a boundary's content root: the box's own style, at the size
/// its parent gave it.
fn content_style(style: &Style, size: taffy::Size<f32>, hidden: bool) -> Style {
    let mut s = style.clone();
    s.position = Position::Relative;
    s.inset = taffy::Rect {
        left: LengthPercentageAuto::auto(),
        right: LengthPercentageAuto::auto(),
        top: LengthPercentageAuto::auto(),
        bottom: LengthPercentageAuto::auto(),
    };
    s.margin = taffy::Rect {
        left: LengthPercentageAuto::length(0.0),
        right: LengthPercentageAuto::length(0.0),
        top: LengthPercentageAuto::length(0.0),
        bottom: LengthPercentageAuto::length(0.0),
    };
    // `size` is the border box, whatever the box's own sizing is.
    s.box_sizing = BoxSizing::BorderBox;
    s.size = taffy::Size {
        width: Dimension::length(size.width),
        height: Dimension::length(size.height),
    };
    let auto = taffy::Size {
        width: LengthPercentageAuto::auto(),
        height: LengthPercentageAuto::auto(),
    };
    s.min_size = auto;
    s.max_size = auto;
    s.aspect_ratio = None;
    s.align_self = None;
    s.justify_self = None;
    s.grid_row = Style::default().grid_row;
    s.grid_column = Style::default().grid_column;
    if hidden {
        s.display = Display::None;
    }
    s
}

impl LayoutTree {
    /// An empty tree.
    #[must_use]
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        LayoutTree {
            taffy,
            entries: HashMap::new(),
            owners: HashMap::new(),
            pending: HashSet::new(),
            moved: HashSet::new(),
            available: None,
            boundaries: true,
            measure_calls: Cell::new(0),
            stats: LayoutStats::default(),
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the tree has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry(&self, node: NodeId) -> Result<&Entry, LayoutError> {
        self.entries
            .get(&node)
            .ok_or(LayoutError(taffy::TaffyError::InvalidInputNode(node)))
    }

    /// The Taffy id of a live node.
    fn id(&self, node: LayoutNode) -> Result<NodeId, LayoutError> {
        self.entry(node.0).map(|_| node.0)
    }

    /// The Taffy node that holds the children of `node`.
    fn holder(&self, node: NodeId) -> NodeId {
        self.entries
            .get(&node)
            .and_then(|e| e.content)
            .unwrap_or(node)
    }

    /// The node whose child `node` is.
    fn parent_of(&self, node: NodeId) -> Option<NodeId> {
        let p = self.taffy.parent(node)?;
        Some(self.owners.get(&p).copied().unwrap_or(p))
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
        let (placed, offset) = if self.boundaries {
            placement(&style)
        } else {
            (style.clone(), Vec2::ZERO)
        };
        let id = if measure == Measure::None {
            self.taffy.new_leaf(placed)?
        } else {
            self.taffy.new_leaf_with_context(placed, measure)?
        };
        self.entries.insert(
            id,
            Entry {
                style,
                content: None,
                offset,
                rect: Rect::ZERO,
                base: Vec2::ZERO,
                frame: Vec2::ZERO,
                hidden: false,
            },
        );
        Ok(LayoutNode(id))
    }

    /// Remove a node (children are detached, not removed).
    ///
    /// # Errors
    /// Taffy errors (unknown node).
    pub fn remove(&mut self, node: LayoutNode) -> Result<(), LayoutError> {
        let id = self.id(node)?;
        let parent = self.parent_of(id);
        if let Some(content) = self.entries.get(&id).and_then(|e| e.content) {
            self.taffy.remove(content)?;
            self.owners.remove(&content);
        }
        self.taffy.remove(id)?;
        self.entries.remove(&id);
        self.pending.remove(&id);
        self.moved.remove(&id);
        if let Some(p) = parent {
            self.reshape(p)?;
            self.dirty_ancestors(self.holder(p))?;
        }
        Ok(())
    }

    /// Set a node's style; returns whether it changed.
    ///
    /// A change of `left`/`top` alone on a box placed by them only moves
    /// it: nothing is dirtied.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn set_style(&mut self, node: LayoutNode, style: &Style) -> Result<bool, LayoutError> {
        let id = self.id(node)?;
        let entry = self.entry(id)?;
        if entry.style == *style {
            return Ok(false);
        }
        let (placed, offset) = if self.boundaries {
            placement(style)
        } else {
            (style.clone(), Vec2::ZERO)
        };
        let moved = entry.offset != offset;
        let resized = *self.taffy.style(id)? != placed;
        if resized {
            self.taffy.set_style(id, placed)?;
            self.dirty_ancestors(id)?;
        }
        let Some(entry) = self.entries.get_mut(&id) else {
            return Ok(false);
        };
        entry.style = style.clone();
        entry.offset = offset;
        if moved {
            self.moved.insert(id);
        }
        if resized && entry.content.is_some() {
            // The content root is brought up to date when next laid out.
            self.pending.insert(id);
        }
        self.reshape(id)?;
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
        let mut seen = HashSet::with_capacity(ids.len());
        if !ids.iter().all(|c| seen.insert(*c)) {
            return Err(LayoutError(taffy::TaffyError::InvalidInputNode(id)));
        }
        let holder = self.holder(id);
        if self.taffy.children(holder)? == ids {
            return Ok(false);
        }
        let old_parents: HashSet<NodeId> = ids
            .iter()
            .filter_map(|c| self.parent_of(*c))
            .filter(|p| *p != id)
            .collect();
        self.taffy.set_children(holder, &ids)?;
        for p in old_parents {
            self.reshape(p)?;
            self.dirty_ancestors(self.holder(p))?;
        }
        self.dirty_ancestors(holder)?;
        self.reshape(id)?;
        Ok(true)
    }

    /// Give a node a content tree of its own, or take it away, as its style
    /// and children require.
    fn reshape(&mut self, id: NodeId) -> Result<(), LayoutError> {
        let entry = self.entry(id)?;
        let content = entry.content;
        let want = self.boundaries
            && is_boundary(&entry.style)
            && !self.taffy.children(content.unwrap_or(id))?.is_empty();
        match content {
            None if want => {
                let style = content_style(&entry.style, taffy::Size::zero(), false);
                let children = self.taffy.children(id)?;
                self.taffy.set_children(id, &[])?;
                let content = self.taffy.new_leaf(style)?;
                self.taffy.set_children(content, &children)?;
                self.owners.insert(content, id);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.content = Some(content);
                }
                self.pending.insert(id);
                self.dirty_ancestors(id)?;
            }
            Some(content) if !want => {
                let children = self.taffy.children(content)?;
                self.taffy.set_children(content, &[])?;
                self.taffy.remove(content)?;
                self.owners.remove(&content);
                if let Some(e) = self.entries.get_mut(&id) {
                    e.content = None;
                }
                self.pending.remove(&id);
                self.taffy.set_children(id, &children)?;
                self.dirty_ancestors(id)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether the node needs layout (for a layout boundary, in its parent
    /// or inside).
    ///
    /// # Errors
    /// Taffy errors.
    pub fn is_dirty(&self, node: LayoutNode) -> Result<bool, LayoutError> {
        let id = self.id(node)?;
        Ok(self.taffy.dirty(id)? || self.pending.contains(&id))
    }

    /// Whether [`LayoutTree::compute`] of `root` has anything to do.
    #[must_use]
    pub fn needs_compute(&self, root: LayoutNode) -> bool {
        let Ok(root) = self.id(root) else {
            return false;
        };
        !self.moved.is_empty()
            || self.taffy.dirty(root).unwrap_or(false)
            || self
                .pending
                .iter()
                .any(|&n| self.depth_under(n, root).is_some())
    }

    /// The node's parent, if attached.
    ///
    /// # Errors
    /// Unknown node.
    pub fn parent(&self, node: LayoutNode) -> Result<Option<LayoutNode>, LayoutError> {
        Ok(self.parent_of(self.id(node)?).map(LayoutNode))
    }

    /// The node's children.
    ///
    /// # Errors
    /// Unknown node.
    pub fn children(&self, node: LayoutNode) -> Result<Vec<LayoutNode>, LayoutError> {
        Ok(self
            .taffy
            .children(self.holder(self.id(node)?))?
            .into_iter()
            .map(LayoutNode)
            .collect())
    }

    /// Clear the cache of `node` and every ancestor in its tree; a content
    /// tree that is reached this way is pending.
    ///
    /// Taffy's `mark_dirty` stops at the first node whose cache is already
    /// empty, assuming its ancestors are dirty too. Nodes under a
    /// `display: none` box break that: hidden layout clears their cache and
    /// never refills it, while the hidden box itself is cached, so a change
    /// under it was never laid out (stale rects for moved or new nodes).
    fn dirty_ancestors(&mut self, node: NodeId) -> Result<(), LayoutError> {
        let mut cur = node;
        loop {
            self.taffy.mark_dirty(cur)?;
            let Some(parent) = self.taffy.parent(cur) else {
                break;
            };
            cur = parent;
        }
        if let Some(owner) = self.owners.get(&cur) {
            self.pending.insert(*owner);
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
            cur = self.parent_of(n);
        }
        false
    }

    /// Whether `node` is `root` or below it.
    fn depth_under(&self, node: NodeId, root: NodeId) -> Option<usize> {
        let mut cur = Some(node);
        let mut depth = 0;
        while let Some(n) = cur {
            if n == root {
                return Some(depth);
            }
            depth += 1;
            cur = self.parent_of(n);
        }
        None
    }

    /// Bring the rects under `root` up to date for `available` space: lay
    /// out the main tree if it is dirty, the contents of the boundaries
    /// that changed, and re-place the boxes that only moved. Clean subtrees
    /// whose constraints did not change are served from Taffy's cache.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn compute(
        &mut self,
        root: LayoutNode,
        available: Size,
    ) -> Result<LayoutOutcome, LayoutError> {
        let root = self.id(root)?;
        let mut out = LayoutOutcome::default();
        if self.taffy.dirty(root)? || self.available != Some(available) {
            self.available = Some(available);
            self.stats.computes += 1;
            self.run(
                root,
                taffy::Size {
                    width: AvailableSpace::Definite(available.width),
                    height: AvailableSpace::Definite(available.height),
                },
            )?;
            out.ran = true;
            out.changed.push(LayoutNode(root));
            self.round(vec![(root, None, false)], Vec::new(), &mut out)?;
        }
        // Boundaries the pass above did not reach, outermost first. Those of
        // detached subtrees stay pending until they are attached again.
        let mut pending: Vec<(usize, NodeId)> = self
            .pending
            .iter()
            .filter_map(|&n| Some((self.depth_under(n, root)?, n)))
            .collect();
        pending.sort_unstable_by_key(|(depth, n)| (*depth, u64::from(*n)));
        for (_, owner) in pending {
            self.compute_boundary(owner, &mut out)?;
        }
        for node in std::mem::take(&mut self.moved) {
            self.place(node, &mut out)?;
        }
        Ok(out)
    }

    /// Run Taffy on the tree under `root`.
    fn run(
        &mut self,
        root: NodeId,
        available: taffy::Size<AvailableSpace>,
    ) -> Result<(), LayoutError> {
        let counter = &self.measure_calls;
        self.taffy
            .compute_layout_with_measure(root, available, |inputs, _id, ctx, style| {
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
            })?;
        Ok(())
    }

    /// Lay out the contents of boundary `owner` if they changed, or the box
    /// got another size, or was hidden or shown.
    fn compute_boundary(
        &mut self,
        owner: NodeId,
        out: &mut LayoutOutcome,
    ) -> Result<(), LayoutError> {
        let Some(entry) = self.entries.get(&owner) else {
            return Ok(());
        };
        let Some(content) = entry.content else {
            return Ok(());
        };
        let hidden = entry.hidden;
        let size = self.taffy.unrounded_layout(owner).size;
        let style = content_style(&entry.style, size, hidden);
        self.pending.remove(&owner);
        if *self.taffy.style(content)? != style {
            self.taffy.set_style(content, style)?;
        }
        if !self.taffy.dirty(content)? {
            return Ok(());
        }
        self.stats.boundary_computes += 1;
        self.run(
            content,
            taffy::Size {
                width: AvailableSpace::Definite(size.width),
                height: AvailableSpace::Definite(size.height),
            },
        )?;
        out.ran = true;
        out.changed.push(LayoutNode(owner));
        let stack = self
            .taffy
            .children(content)?
            .into_iter()
            .rev()
            .map(|c| (c, Some(0), hidden))
            .collect();
        self.round(stack, vec![(content, Vec2::ZERO)], out)
    }

    /// Pixel-snap the nodes of `stack` and their subtrees, like Taffy's own
    /// rounding (edges are rounded in a shared frame, so adjacent boxes stay
    /// flush), but walking real children instead of hoisted lists.
    ///
    /// Unlike Taffy, every out-of-flow box starts a new frame: its size is
    /// rounded on its own and its subtree is rounded relative to its origin.
    /// Moving such a box by a fraction of a pixel therefore never shifts or
    /// resizes its contents relative to it (a dragged node card and its
    /// labels move as one), and the contents of a boundary can be rounded
    /// without knowing where it is.
    fn round(
        &mut self,
        mut stack: Vec<(NodeId, Option<usize>, bool)>,
        mut chain: Vec<(NodeId, Vec2)>,
        out: &mut LayoutOutcome,
    ) -> Result<(), LayoutError> {
        // `chain`: (node, unrounded origin in its frame) for the ancestors.
        // `stack`: (node, depth of its parent in `chain`, hidden above).
        let mut boundaries = Vec::new();
        while let Some((node, parent_depth, hidden_above)) = stack.pop() {
            chain.truncate(parent_depth.map_or(0, |d| d + 1));
            let parent = chain.last().map_or(Vec2::ZERO, |&(_, p)| p);
            // An out-of-flow box is positioned relative to its containing
            // block: the nearest ancestor that hoisted it (the compute root
            // claims whatever no positioned ancestor did).
            let style = self.taffy.style(node)?;
            let out_of_flow =
                style.position.is_out_of_flow() && style.display != taffy::Display::None;
            let hidden = hidden_above || style.display == taffy::Display::None;
            let base = if out_of_flow && !chain.is_empty() {
                chain
                    .iter()
                    .rev()
                    .find(|(a, _)| {
                        self.taffy
                            .hoisted_children(*a)
                            .is_ok_and(|h| h.contains(&node))
                    })
                    .map_or(parent, |&(_, p)| p)
            } else {
                parent
            };
            let l = self.taffy.unrounded_layout(node);
            let new_frame = out_of_flow && !chain.is_empty();
            let Some(entry) = self.entries.get_mut(&node) else {
                continue;
            };
            let offset = if hidden { Vec2::ZERO } else { entry.offset };
            let at = base + Vec2::new(l.location.x, l.location.y) + offset;
            let (width, height) = if new_frame {
                (round(l.size.width), round(l.size.height))
            } else {
                (
                    round(at.x + l.size.width) - round(at.x),
                    round(at.y + l.size.height) - round(at.y),
                )
            };
            entry.rect = Rect::new(
                round(at.x) - round(parent.x),
                round(at.y) - round(parent.y),
                width,
                height,
            );
            entry.base = base;
            entry.frame = parent;
            entry.hidden = hidden;
            if entry.content.is_some() {
                boundaries.push(node);
                continue;
            }
            let depth = chain.len();
            chain.push((node, if new_frame { Vec2::ZERO } else { at }));
            let children = self.taffy.children(node)?;
            stack.extend(children.into_iter().rev().map(|c| (c, Some(depth), hidden)));
        }
        for owner in boundaries {
            self.compute_boundary(owner, out)?;
        }
        Ok(())
    }

    /// Re-place a box that moved: its size and its subtree stay.
    fn place(&mut self, node: NodeId, out: &mut LayoutOutcome) -> Result<(), LayoutError> {
        let Some(entry) = self.entries.get_mut(&node) else {
            return Ok(());
        };
        if entry.hidden {
            return Ok(());
        }
        let l = self.taffy.unrounded_layout(node);
        let at = entry.base + Vec2::new(l.location.x, l.location.y) + entry.offset;
        let (x, y) = (
            round(at.x) - round(entry.frame.x),
            round(at.y) - round(entry.frame.y),
        );
        if (x, y) != (entry.rect.x, entry.rect.y) {
            entry.rect.x = x;
            entry.rect.y = y;
            self.stats.moves += 1;
            out.changed.push(LayoutNode(node));
        }
        Ok(())
    }

    /// The node's border box relative to its parent's border box.
    ///
    /// # Errors
    /// Taffy errors.
    pub fn rect(&self, node: LayoutNode) -> Result<Rect, LayoutError> {
        self.entry(node.0).map(|e| e.rect)
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

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
