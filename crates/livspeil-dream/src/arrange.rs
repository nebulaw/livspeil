// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Positions for the nodes a room does not place.
//!
//! Nodes with a position keep it. The others are laid out in columns by how
//! many links lead to them, below everything already placed. Nothing is
//! written to the graph: a position the user did not choose is not an edit.

use livspeil_ui_widgets::NODE_WIDTH;

use crate::shown::{Shown, ShownLink};

/// Space between columns and between nodes of a column.
const GAP: f32 = 60.0;

/// The column of each node: the longest chain of links leading to it.
fn depths(count: usize, links: &[ShownLink]) -> Vec<usize> {
    let mut depth = vec![0_usize; count];
    // Longest path by relaxation; value links form no cycles, and the bound
    // keeps event loops (which may) from running away.
    for _ in 0..count {
        let mut changed = false;
        for link in links {
            let (from, to) = (link.from.0, link.to.0);
            if from != to && depth[to] < depth[from] + 1 && depth[from] + 1 < count {
                depth[to] = depth[from] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    depth
}

/// A position for every node: `placed[i]` if set, else arranged. `heights`
/// are the cards' heights.
#[must_use]
pub fn arrange(
    nodes: &[Shown],
    links: &[ShownLink],
    placed: &[Option<[f32; 2]>],
    heights: &[f32],
) -> Vec<[f32; 2]> {
    let top = placed
        .iter()
        .zip(heights)
        .filter_map(|(p, h)| p.map(|p| p[1] + h + GAP))
        .fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.max(b))))
        .unwrap_or(0.0);
    let left = placed
        .iter()
        .flatten()
        .map(|p| p[0])
        .fold(None, |a: Option<f32>, b| Some(a.map_or(b, |a| a.min(b))))
        .unwrap_or(0.0);
    let depth = depths(nodes.len(), links);
    let mut next: Vec<f32> = vec![top; nodes.len().max(1)];
    (0..nodes.len())
        .map(|i| {
            if let Some(p) = placed[i] {
                return p;
            }
            #[allow(clippy::cast_precision_loss)]
            let x = left + depth[i] as f32 * (NODE_WIDTH + GAP);
            let y = next[depth[i]];
            next[depth[i]] += heights[i] + GAP / 2.0;
            [x, y]
        })
        .collect()
}
