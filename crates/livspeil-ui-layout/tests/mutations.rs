// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Tree mutations between layout passes: regression tests for Taffy 0.16's
//! stale hoisted-children lists (see `docs/known-issues.md`) and a
//! randomized test comparing incremental relayout with a from-scratch one.

use livspeil_ui_layout::style::{Dimension, Display, LengthPercentageAuto, Position};
use livspeil_ui_layout::{LayoutNode, LayoutTree, Measure, Rect, Size, Style, taffy};

const VIEWPORT: Size = Size::new(400.0, 300.0);

fn absolute(x: f32, y: f32, w: f32, h: f32) -> Style {
    Style {
        position: Position::Absolute,
        inset: taffy::Rect {
            left: LengthPercentageAuto::length(x),
            right: LengthPercentageAuto::auto(),
            top: LengthPercentageAuto::length(y),
            bottom: LengthPercentageAuto::auto(),
        },
        size: taffy::Size {
            width: Dimension::length(w),
            height: Dimension::length(h),
        },
        ..Style::default()
    }
}

/// The minimal reproduction: an absolute box loses its only child, which
/// is itself absolute. With Taffy's own rounding the next layout panicked
/// with "invalid SlotMap key".
#[test]
fn removing_last_absolute_child_of_absolute_parent() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let overlay = t
        .create(absolute(10.0, 10.0, 100.0, 50.0), Measure::None)
        .unwrap();
    let popup = t
        .create(absolute(5.0, 5.0, 20.0, 20.0), Measure::None)
        .unwrap();
    t.set_children(root, &[overlay]).unwrap();
    t.set_children(overlay, &[popup]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(popup).unwrap(), Rect::new(5.0, 5.0, 20.0, 20.0));

    t.remove(popup).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(overlay).unwrap(), Rect::new(10.0, 10.0, 100.0, 50.0));
    assert!(t.rect(popup).is_err(), "removed node is unknown");
}

/// Same, but the parent is hidden (`display: none`) before the child goes:
/// hidden layout does not rewrite the hoisted list either.
#[test]
fn removing_absolute_child_of_hidden_absolute_parent() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let overlay = t
        .create(absolute(10.0, 10.0, 100.0, 50.0), Measure::None)
        .unwrap();
    let popup = t
        .create(absolute(5.0, 5.0, 20.0, 20.0), Measure::None)
        .unwrap();
    let other = t
        .create(absolute(1.0, 1.0, 2.0, 2.0), Measure::None)
        .unwrap();
    t.set_children(root, &[overlay]).unwrap();
    t.set_children(overlay, &[popup, other]).unwrap();
    t.compute(root, VIEWPORT).unwrap();

    let hidden = Style {
        display: Display::None,
        ..absolute(10.0, 10.0, 100.0, 50.0)
    };
    t.set_style(overlay, &hidden).unwrap();
    t.remove(popup).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(overlay).unwrap(), Rect::ZERO);
    assert_eq!(t.rect(other).unwrap(), Rect::ZERO);
}

/// A box moved out of an absolute parent that becomes a leaf is positioned
/// by its new parent only (the stale list must not move it again).
#[test]
fn reparenting_out_of_absolute_parent() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let a = t
        .create(absolute(50.0, 50.0, 100.0, 100.0), Measure::None)
        .unwrap();
    let b = t
        .create(absolute(200.0, 20.0, 100.0, 100.0), Measure::None)
        .unwrap();
    let c = t
        .create(absolute(5.0, 5.0, 10.0, 10.0), Measure::None)
        .unwrap();
    t.set_children(root, &[a, b]).unwrap();
    t.set_children(a, &[c]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    t.set_children(b, &[c]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.children(a).unwrap(), Vec::new());
    assert_eq!(t.parent(c).unwrap(), Some(b));
    assert_eq!(t.rect(c).unwrap(), Rect::new(5.0, 5.0, 10.0, 10.0));
}

/// A box moved under a hidden subtree is hidden on the next layout. Taffy
/// stops dirty propagation at nodes under `display: none` (their cache is
/// always empty), so the hidden ancestor stayed cached.
#[test]
fn moving_into_hidden_subtree() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let hidden = t
        .create(
            Style {
                display: Display::None,
                ..Style::default()
            },
            Measure::None,
        )
        .unwrap();
    let inner = t.create(Style::default(), Measure::None).unwrap();
    let leaf = t
        .create(Style::default(), Measure::Fixed(Size::new(30.0, 20.0)))
        .unwrap();
    t.set_children(root, &[hidden, leaf]).unwrap();
    t.set_children(hidden, &[inner]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(leaf).unwrap(), Rect::new(0.0, 0.0, 30.0, 20.0));

    t.set_children(inner, &[leaf]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(leaf).unwrap(), Rect::ZERO);
}

/// Stale ids and cycles are errors, never panics.
#[test]
fn invalid_operations_are_errors() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let a = t.create(Style::default(), Measure::None).unwrap();
    t.set_children(root, &[a]).unwrap();
    assert!(t.set_children(a, &[root]).is_err(), "cycle");
    assert!(t.set_children(a, &[a]).is_err(), "self");
    let b = t.create(Style::default(), Measure::None).unwrap();
    assert!(t.set_children(root, &[b, b]).is_err(), "duplicate");
    t.remove(b).unwrap();
    assert!(t.remove(b).is_err());
    assert!(t.set_children(root, &[a, b]).is_err());
    assert!(t.set_style(b, &Style::default()).is_err());
    assert!(t.compute(b, VIEWPORT).is_err());
    t.compute(root, VIEWPORT).unwrap();
}

/// SplitMix64: a tiny deterministic generator (no dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap()
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn len(&mut self, max: u32) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let v = (self.next() % u64::from(max * 4)) as f32;
        v / 4.0
    }
}

/// Model of one node: what a from-scratch tree is built from.
#[derive(Clone)]
struct Model {
    style: Style,
    measure: Measure,
    children: Vec<usize>,
    parent: Option<usize>,
}

fn random_style(rng: &mut Rng) -> Style {
    let position = match rng.below(3) {
        0 => Position::Relative,
        1 => Position::Absolute,
        _ => Position::Static,
    };
    let display = match rng.below(10) {
        0..=3 => Display::Flex,
        4..=5 => Display::Grid,
        6..=8 => Display::Block,
        _ => Display::None,
    };
    let dim = |rng: &mut Rng| {
        if rng.chance(40) {
            Dimension::auto()
        } else {
            Dimension::length(rng.len(120))
        }
    };
    let inset = |rng: &mut Rng| {
        if rng.chance(30) {
            LengthPercentageAuto::auto()
        } else {
            LengthPercentageAuto::length(rng.len(60))
        }
    };
    Style {
        display,
        position,
        flex_direction: if rng.chance(50) {
            taffy::FlexDirection::Row
        } else {
            taffy::FlexDirection::Column
        },
        flex_grow: if rng.chance(30) { 1.0 } else { 0.0 },
        size: taffy::Size {
            width: dim(rng),
            height: dim(rng),
        },
        inset: taffy::Rect {
            left: inset(rng),
            right: inset(rng),
            top: inset(rng),
            bottom: inset(rng),
        },
        padding: taffy::Rect::length(rng.len(4)),
        ..Style::default()
    }
}

fn random_measure(rng: &mut Rng) -> Measure {
    match rng.below(3) {
        0 => Measure::None,
        1 => Measure::Fixed(Size::new(rng.len(40), rng.len(20))),
        _ => Measure::TextPlaceholder {
            chars: u32::try_from(rng.below(40)).unwrap(),
            font_size: 13.0,
            line_height: 16.0,
        },
    }
}

fn is_descendant(model: &[Option<Model>], node: usize, of: usize) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n == of {
            return true;
        }
        cur = model[n].as_ref().and_then(|m| m.parent);
    }
    false
}

fn subtree(model: &[Option<Model>], node: usize, out: &mut Vec<usize>) {
    out.push(node);
    if let Some(m) = &model[node] {
        for &c in &m.children {
            subtree(model, c, out);
        }
    }
}

/// Lay out the model from scratch and return every attached node's rect.
fn fresh(model: &[Option<Model>]) -> Vec<(usize, Rect)> {
    let mut t = LayoutTree::new();
    let ids: Vec<Option<LayoutNode>> = model
        .iter()
        .map(|m| {
            m.as_ref()
                .map(|m| t.create(m.style.clone(), m.measure.clone()).unwrap())
        })
        .collect();
    for (m, id) in model.iter().zip(&ids) {
        if let (Some(m), Some(id)) = (m, id) {
            let children: Vec<LayoutNode> = m.children.iter().map(|&c| ids[c].unwrap()).collect();
            t.set_children(*id, &children).unwrap();
        }
    }
    let root = ids[0].unwrap();
    t.compute(root, VIEWPORT).unwrap();
    let mut nodes = Vec::new();
    subtree(model, 0, &mut nodes);
    nodes
        .into_iter()
        .map(|n| (n, t.rect(ids[n].unwrap()).unwrap()))
        .collect()
}

#[test]
fn random_mutations_match_fresh_layout() {
    for seed in 0..300u64 {
        let mut rng = Rng(seed);
        let mut t = LayoutTree::new();
        let mut model: Vec<Option<Model>> = Vec::new();
        let mut ids: Vec<Option<LayoutNode>> = Vec::new();

        let root_style = Style {
            position: if rng.chance(50) {
                Position::Relative
            } else {
                Position::Static
            },
            ..random_style(&mut rng)
        };
        let root_style = Style {
            display: Display::Flex,
            ..root_style
        };
        ids.push(Some(t.create(root_style.clone(), Measure::None).unwrap()));
        model.push(Some(Model {
            style: root_style,
            measure: Measure::None,
            children: Vec::new(),
            parent: None,
        }));

        for step in 0..60 {
            let live: Vec<usize> = (0..model.len()).filter(|&i| model[i].is_some()).collect();
            let attached: Vec<usize> = {
                let mut v = Vec::new();
                subtree(&model, 0, &mut v);
                v
            };
            let op = rng.below(10);
            match op {
                // Insert a new node under a random attached node.
                0..=3 => {
                    let parent = attached[rng.below(attached.len())];
                    let style = random_style(&mut rng);
                    let measure = random_measure(&mut rng);
                    let id = t.create(style.clone(), measure.clone()).unwrap();
                    let n = model.len();
                    ids.push(Some(id));
                    model.push(Some(Model {
                        style,
                        measure,
                        children: Vec::new(),
                        parent: Some(parent),
                    }));
                    let pm = model[parent].as_mut().unwrap();
                    let at = rng.below(pm.children.len() + 1);
                    pm.children.insert(at, n);
                    let children: Vec<LayoutNode> =
                        pm.children.iter().map(|&c| ids[c].unwrap()).collect();
                    t.set_children(ids[parent].unwrap(), &children).unwrap();
                }
                // Remove a subtree (as the UI core does: children first,
                // or parent first; both must work).
                4..=5 if attached.len() > 1 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let mut doomed = Vec::new();
                    subtree(&model, node, &mut doomed);
                    let parent = model[node].as_ref().unwrap().parent.unwrap();
                    let pm = model[parent].as_mut().unwrap();
                    pm.children.retain(|&c| c != node);
                    let children: Vec<LayoutNode> =
                        pm.children.iter().map(|&c| ids[c].unwrap()).collect();
                    let detach_first = rng.chance(50);
                    if detach_first {
                        t.set_children(ids[parent].unwrap(), &children).unwrap();
                    }
                    if rng.chance(50) {
                        doomed.reverse();
                    }
                    for d in doomed {
                        t.remove(ids[d].take().unwrap()).unwrap();
                        model[d] = None;
                    }
                    if !detach_first {
                        assert!(!t.set_children(ids[parent].unwrap(), &children).unwrap());
                    }
                }
                // Reparent a node to a non-descendant.
                6 if attached.len() > 2 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let target = attached[rng.below(attached.len())];
                    if is_descendant(&model, target, node) {
                        continue;
                    }
                    let old = model[node].as_ref().unwrap().parent.unwrap();
                    model[old].as_mut().unwrap().children.retain(|&c| c != node);
                    if rng.chance(50) || old == target {
                        let children: Vec<LayoutNode> = model[old]
                            .as_ref()
                            .unwrap()
                            .children
                            .iter()
                            .map(|&c| ids[c].unwrap())
                            .collect();
                        t.set_children(ids[old].unwrap(), &children).unwrap();
                    }
                    let tm = model[target].as_mut().unwrap();
                    let at = rng.below(tm.children.len() + 1);
                    tm.children.insert(at, node);
                    let children: Vec<LayoutNode> =
                        tm.children.iter().map(|&c| ids[c].unwrap()).collect();
                    t.set_children(ids[target].unwrap(), &children).unwrap();
                    model[node].as_mut().unwrap().parent = Some(target);
                }
                // Change a style (keep the root a flex container).
                7..=8 => {
                    let node = live[rng.below(live.len())];
                    let mut style = random_style(&mut rng);
                    if node == 0 {
                        style.display = Display::Flex;
                        style.position = Position::Relative;
                    }
                    t.set_style(ids[node].unwrap(), &style).unwrap();
                    model[node].as_mut().unwrap().style = style;
                }
                // Change a measure.
                _ => {
                    let node = live[rng.below(live.len())];
                    let measure = random_measure(&mut rng);
                    t.set_measure(ids[node].unwrap(), measure.clone()).unwrap();
                    model[node].as_mut().unwrap().measure = measure;
                }
            }

            if rng.chance(40) {
                t.compute(ids[0].unwrap(), VIEWPORT).unwrap();
                for (n, expected) in fresh(&model) {
                    let got = t.rect(ids[n].unwrap()).unwrap();
                    assert_eq!(
                        got, expected,
                        "seed {seed} step {step}: node {n} incremental {got} vs fresh {expected}"
                    );
                }
            }
        }
    }
}

/// Moving an out-of-flow box by fractions of a pixel keeps its size and its
/// contents' offsets, so a dragged card and its labels move as one.
#[test]
fn contents_of_a_moving_absolute_box_do_not_shift() {
    let mut t = LayoutTree::new();
    let root = t.create(Style::default(), Measure::None).unwrap();
    let card = t
        .create(absolute(0.0, 0.0, 100.0, 50.5), Measure::None)
        .unwrap();
    let label = t
        .create(absolute(10.0, 6.5, 40.0, 15.0), Measure::None)
        .unwrap();
    t.set_children(root, &[card]).unwrap();
    t.set_children(card, &[label]).unwrap();

    let mut seen = None;
    for step in 0..40 {
        let at = 20.0 + step as f32 * 0.1;
        t.set_style(card, &absolute(at, at, 100.0, 50.5)).unwrap();
        t.compute(root, VIEWPORT).unwrap();
        let card_rect = t.rect(card).unwrap();
        let now = (t.rect(label).unwrap(), card_rect.width, card_rect.height);
        assert_eq!(*seen.get_or_insert(now), now, "card at {at}");
    }
}
