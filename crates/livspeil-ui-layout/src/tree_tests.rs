// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Layout boundaries: what they lay out, and that an incrementally
//! maintained tree with them equals plain Taffy built from scratch.

use super::*;
use crate::geometry::Rect;
use taffy::FlexDirection;

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

fn leaf(w: f32, h: f32) -> Style {
    Style {
        size: taffy::Size {
            width: Dimension::length(w),
            height: Dimension::length(h),
        },
        ..Style::default()
    }
}

/// A root with `n` absolute cards of two rows each.
fn cards(n: usize) -> (LayoutTree, LayoutNode, Vec<(LayoutNode, [LayoutNode; 2])>) {
    let mut t = LayoutTree::new();
    let root = t.create(leaf(400.0, 300.0), Measure::None).unwrap();
    let cards: Vec<_> = (0..n)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let at = i as f32 * 10.0;
            let card = Style {
                flex_direction: FlexDirection::Column,
                ..absolute(at, at, 100.0, 60.0)
            };
            let card = t.create(card, Measure::None).unwrap();
            let rows = [0, 1].map(|_| t.create(leaf(50.0, 20.0), Measure::None).unwrap());
            t.set_children(card, &rows).unwrap();
            (card, rows)
        })
        .collect();
    let ids: Vec<_> = cards.iter().map(|c| c.0).collect();
    t.set_children(root, &ids).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    (t, root, cards)
}

#[test]
fn a_change_inside_a_boundary_lays_out_only_it() {
    let (mut t, root, cards) = cards(20);
    let before = t.stats();
    assert!(!t.needs_compute(root));
    let (card, [first, second]) = cards[7];
    t.set_style(first, &leaf(50.0, 30.0)).unwrap();
    assert!(t.needs_compute(root));
    assert!(t.is_dirty(card).unwrap());
    assert!(!t.is_dirty(root).unwrap());
    let out = t.compute(root, VIEWPORT).unwrap();
    assert!(out.ran);
    assert_eq!(out.changed, vec![card]);
    let after = t.stats();
    assert_eq!(after.computes, before.computes, "no main layout");
    assert_eq!(after.boundary_computes, before.boundary_computes + 1);
    assert_eq!(t.rect(second).unwrap(), Rect::new(0.0, 30.0, 50.0, 20.0));
}

#[test]
fn moving_a_boundary_runs_no_layout() {
    let (mut t, root, cards) = cards(20);
    let before = t.stats();
    let (card, [first, _]) = cards[3];
    let inside = t.rect(first).unwrap();
    let mut moved = absolute(200.4, 100.0, 100.0, 60.0);
    moved.flex_direction = FlexDirection::Column;
    assert!(t.set_style(card, &moved).unwrap());
    assert!(!t.is_dirty(root).unwrap() && !t.is_dirty(card).unwrap());
    assert!(t.needs_compute(root));
    let out = t.compute(root, VIEWPORT).unwrap();
    assert!(!out.ran);
    assert_eq!(out.changed, vec![card]);
    assert_eq!(t.rect(card).unwrap(), Rect::new(200.0, 100.0, 100.0, 60.0));
    assert_eq!(t.rect(first).unwrap(), inside);
    let after = t.stats();
    assert_eq!(
        (after.computes, after.boundary_computes, after.measure_calls),
        (
            before.computes,
            before.boundary_computes,
            before.measure_calls
        )
    );
    assert_eq!(after.moves, before.moves + 1);
    assert!(!t.needs_compute(root));
}

#[test]
fn nested_boundaries_stop_at_the_innermost() {
    let mut t = LayoutTree::new();
    let root = t.create(leaf(400.0, 300.0), Measure::None).unwrap();
    let outer = t
        .create(absolute(10.0, 10.0, 200.0, 200.0), Measure::None)
        .unwrap();
    let inner = t
        .create(absolute(5.0, 5.0, 100.0, 100.0), Measure::None)
        .unwrap();
    let child = t.create(leaf(10.0, 10.0), Measure::None).unwrap();
    t.set_children(root, &[outer]).unwrap();
    t.set_children(outer, &[inner]).unwrap();
    t.set_children(inner, &[child]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    let before = t.stats();
    t.set_style(child, &leaf(20.0, 10.0)).unwrap();
    let out = t.compute(root, VIEWPORT).unwrap();
    assert_eq!(out.changed, vec![inner]);
    assert_eq!(t.stats().boundary_computes, before.boundary_computes + 1);
    assert_eq!(t.parent(child).unwrap(), Some(inner));
    assert_eq!(t.children(outer).unwrap(), vec![inner]);
    // Resizing the outer box does not disturb the inner one.
    t.set_style(outer, &absolute(10.0, 10.0, 300.0, 200.0))
        .unwrap();
    let before = t.stats();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(inner).unwrap(), Rect::new(5.0, 5.0, 100.0, 100.0));
    assert_eq!(t.stats().boundary_computes, before.boundary_computes + 1);
}

/// SplitMix64.
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

    fn quarter(&mut self, max: u32) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let v = (self.next() % u64::from(max * 4)) as f32;
        v / 4.0
    }
}

/// Mostly boxes that are boundaries or nearly so, with fractional places.
fn random_style(rng: &mut Rng) -> Style {
    let position = match rng.below(8) {
        0..=4 => Position::Absolute,
        5 | 6 => Position::Relative,
        _ => Position::Static,
    };
    let display = match rng.below(12) {
        0..=5 => Display::Flex,
        6 => Display::Grid,
        7..=10 => Display::Block,
        _ => Display::None,
    };
    let dim = |rng: &mut Rng| match rng.below(10) {
        0 | 1 => Dimension::auto(),
        2 | 3 => Dimension::percent(0.25 * (1 + rng.below(4)) as f32),
        _ => Dimension::length(rng.quarter(120)),
    };
    let near = |rng: &mut Rng| {
        if rng.below(6) == 0 {
            LengthPercentageAuto::auto()
        } else {
            LengthPercentageAuto::length(rng.quarter(60))
        }
    };
    let far = |rng: &mut Rng| {
        if rng.below(6) == 0 {
            LengthPercentageAuto::length(rng.quarter(60))
        } else {
            LengthPercentageAuto::auto()
        }
    };
    let padding = if rng.below(10) == 0 {
        LengthPercentage::percent(0.05)
    } else {
        LengthPercentage::length(rng.quarter(4))
    };
    Style {
        display,
        position,
        flex_direction: if rng.below(2) == 0 {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        },
        flex_grow: if rng.below(4) == 0 { 1.0 } else { 0.0 },
        size: taffy::Size {
            width: dim(rng),
            height: dim(rng),
        },
        inset: taffy::Rect {
            left: near(rng),
            right: far(rng),
            top: near(rng),
            bottom: far(rng),
        },
        padding: taffy::Rect {
            left: padding,
            right: padding,
            top: padding,
            bottom: padding,
        },
        ..Style::default()
    }
}

#[derive(Clone)]
struct Node {
    style: Style,
    measure: Measure,
    children: Vec<usize>,
    parent: Option<usize>,
}

fn subtree(model: &[Option<Node>], node: usize, out: &mut Vec<usize>) {
    out.push(node);
    for &c in &model[node].as_ref().unwrap().children {
        subtree(model, c, out);
    }
}

/// Every attached rect of the model laid out once without boundaries.
fn plain(model: &[Option<Node>]) -> Vec<(usize, Rect)> {
    let mut t = LayoutTree::new();
    t.boundaries = false;
    let mut nodes = Vec::new();
    subtree(model, 0, &mut nodes);
    let mut ids = HashMap::new();
    for &n in &nodes {
        let m = model[n].as_ref().unwrap();
        ids.insert(n, t.create(m.style.clone(), m.measure.clone()).unwrap());
    }
    for &n in &nodes {
        let m = model[n].as_ref().unwrap();
        let children: Vec<LayoutNode> = m.children.iter().map(|c| ids[c]).collect();
        t.set_children(ids[&n], &children).unwrap();
    }
    t.compute(ids[&0], VIEWPORT).unwrap();
    nodes
        .iter()
        .map(|n| (*n, t.rect(ids[n]).unwrap()))
        .collect()
}

#[test]
fn random_edits_with_boundaries_match_a_plain_layout() {
    let mut boundary_computes = 0;
    let mut moves = 0;
    for seed in 0..600u64 {
        let mut rng = Rng(seed);
        let mut t = LayoutTree::new();
        let root_style = leaf(400.0, 300.0);
        let mut ids = vec![Some(t.create(root_style.clone(), Measure::None).unwrap())];
        let mut model = vec![Some(Node {
            style: root_style,
            measure: Measure::None,
            children: Vec::new(),
            parent: None,
        })];
        // Roots of subtrees taken out of the tree, to be put back.
        let mut detached: Vec<usize> = Vec::new();
        for step in 0..80 {
            let mut attached = Vec::new();
            subtree(&model, 0, &mut attached);
            let kids = |model: &[Option<Node>], ids: &[Option<LayoutNode>], n: usize| {
                model[n]
                    .as_ref()
                    .unwrap()
                    .children
                    .iter()
                    .map(|&c| ids[c].unwrap())
                    .collect::<Vec<_>>()
            };
            match rng.below(14) {
                0..=3 => {
                    let parent = attached[rng.below(attached.len())];
                    let style = random_style(&mut rng);
                    let measure = if rng.below(3) == 0 {
                        Measure::Fixed(Size::new(rng.quarter(40), rng.quarter(20)))
                    } else {
                        Measure::None
                    };
                    ids.push(Some(t.create(style.clone(), measure.clone()).unwrap()));
                    let n = model.len();
                    model.push(Some(Node {
                        style,
                        measure,
                        children: Vec::new(),
                        parent: Some(parent),
                    }));
                    let p = model[parent].as_mut().unwrap();
                    let at = rng.below(p.children.len() + 1);
                    p.children.insert(at, n);
                    t.set_children(ids[parent].unwrap(), &kids(&model, &ids, parent))
                        .unwrap();
                }
                4 if attached.len() > 1 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let mut doomed = Vec::new();
                    subtree(&model, node, &mut doomed);
                    let parent = model[node].as_ref().unwrap().parent.unwrap();
                    model[parent]
                        .as_mut()
                        .unwrap()
                        .children
                        .retain(|&c| c != node);
                    if rng.below(2) == 0 {
                        doomed.reverse();
                    }
                    for d in doomed {
                        t.remove(ids[d].take().unwrap()).unwrap();
                        model[d] = None;
                    }
                }
                5 if attached.len() > 2 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let target = attached[rng.below(attached.len())];
                    let mut under = Vec::new();
                    subtree(&model, node, &mut under);
                    if under.contains(&target) {
                        continue;
                    }
                    let old = model[node].as_ref().unwrap().parent.unwrap();
                    model[old].as_mut().unwrap().children.retain(|&c| c != node);
                    model[target].as_mut().unwrap().children.push(node);
                    model[node].as_mut().unwrap().parent = Some(target);
                    t.set_children(ids[target].unwrap(), &kids(&model, &ids, target))
                        .unwrap();
                }
                // Move a box (only `left`/`top` change).
                6..=8 => {
                    let node = attached[rng.below(attached.len())];
                    if node == 0 {
                        continue;
                    }
                    let m = model[node].as_mut().unwrap();
                    m.style.inset.left = LengthPercentageAuto::length(rng.quarter(80));
                    m.style.inset.top = LengthPercentageAuto::length(rng.quarter(80));
                    t.set_style(ids[node].unwrap(), &m.style).unwrap();
                }
                // Take a subtree out, change something in it later, put
                // it back.
                12 if attached.len() > 1 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let parent = model[node].as_ref().unwrap().parent.unwrap();
                    model[parent]
                        .as_mut()
                        .unwrap()
                        .children
                        .retain(|&c| c != node);
                    model[node].as_mut().unwrap().parent = None;
                    t.set_children(ids[parent].unwrap(), &kids(&model, &ids, parent))
                        .unwrap();
                    detached.push(node);
                }
                13 if !detached.is_empty() => {
                    let node = detached.swap_remove(rng.below(detached.len()));
                    let mut under = Vec::new();
                    subtree(&model, node, &mut under);
                    let inside = under[rng.below(under.len())];
                    let style = random_style(&mut rng);
                    t.set_style(ids[inside].unwrap(), &style).unwrap();
                    model[inside].as_mut().unwrap().style = style;
                    // A layout while it is still out of the tree.
                    t.compute(ids[0].unwrap(), VIEWPORT).unwrap();
                    let target = attached[rng.below(attached.len())];
                    model[target].as_mut().unwrap().children.push(node);
                    model[node].as_mut().unwrap().parent = Some(target);
                    t.set_children(ids[target].unwrap(), &kids(&model, &ids, target))
                        .unwrap();
                }
                9 | 10 if attached.len() > 1 => {
                    let node = attached[1 + rng.below(attached.len() - 1)];
                    let style = random_style(&mut rng);
                    t.set_style(ids[node].unwrap(), &style).unwrap();
                    model[node].as_mut().unwrap().style = style;
                }
                _ => {
                    let node = attached[rng.below(attached.len())];
                    let measure = Measure::Fixed(Size::new(rng.quarter(40), rng.quarter(20)));
                    if model[node].as_ref().unwrap().children.is_empty() {
                        t.set_measure(ids[node].unwrap(), measure.clone()).unwrap();
                        model[node].as_mut().unwrap().measure = measure;
                    }
                }
            }
            if rng.below(100) < 50 {
                let root = ids[0].unwrap();
                t.compute(root, VIEWPORT).unwrap();
                assert!(!t.needs_compute(root), "seed {seed} step {step}");
                for (n, expected) in plain(&model) {
                    let got = t.rect(ids[n].unwrap()).unwrap();
                    assert_eq!(
                        got, expected,
                        "seed {seed} step {step}: node {n} incremental {got} vs plain {expected}"
                    );
                }
            }
        }
        boundary_computes += t.stats().boundary_computes;
        moves += t.stats().moves;
    }
    // The walk really exercised both paths.
    assert!(boundary_computes > 1000, "{boundary_computes}");
    assert!(moves > 500, "{moves}");
}

/// A change inside a subtree that is not attached at the time of a layout
/// is not forgotten: it is laid out once the subtree comes back, also when
/// only a clean boundary above it is attached.
#[test]
fn changes_in_a_detached_subtree_are_laid_out_after_reattaching() {
    let mut t = LayoutTree::new();
    let root = t.create(leaf(400.0, 300.0), Measure::None).unwrap();
    let outer = t
        .create(absolute(10.0, 10.0, 200.0, 200.0), Measure::None)
        .unwrap();
    let inner = t
        .create(absolute(5.0, 5.0, 100.0, 100.0), Measure::None)
        .unwrap();
    let child = t.create(leaf(10.0, 10.0), Measure::None).unwrap();
    t.set_children(root, &[outer]).unwrap();
    t.set_children(outer, &[inner]).unwrap();
    t.set_children(inner, &[child]).unwrap();
    t.compute(root, VIEWPORT).unwrap();

    t.set_children(root, &[]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    t.set_style(child, &leaf(30.0, 10.0)).unwrap();
    assert!(!t.needs_compute(root), "nothing attached changed");
    t.compute(root, VIEWPORT).unwrap();
    t.set_children(root, &[outer]).unwrap();
    t.compute(root, VIEWPORT).unwrap();
    assert_eq!(t.rect(child).unwrap(), Rect::new(0.0, 0.0, 30.0, 10.0));
    assert!(!t.needs_compute(root));
}
