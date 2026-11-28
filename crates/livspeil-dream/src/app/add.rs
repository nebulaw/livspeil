// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Adding a node: the searchable list of types, and where the new node
//! goes.

use livspeil_ui_core::{App, Model, View};
use livspeil_ui_layout::{Point, Vec2};
use livspeil_ui_widgets::{Env, NODE_WIDTH, NodeCanvas, NodeGraphModel, Picker, PickerItem};

use super::{change, focus_canvas};
use crate::canvas::Editor;
use crate::catalog::{self, NodeType};

/// How far above the middle of the view a new card's top goes.
const RISE: f32 = 40.0;
/// How far a new node moves down and right when its place is taken.
const STEP: f32 = 24.0;

fn types() -> Vec<NodeType> {
    match catalog::registry() {
        Ok(registry) => catalog::node_types(&registry),
        Err(e) => {
            log::warn!("no node types to add: {e}");
            Vec::new()
        }
    }
}

/// The list of node types; choosing one adds it in the middle of the view.
pub(super) fn picker(
    app: &mut App,
    env: &Env,
    editor: Model<Editor>,
    canvas: View<NodeCanvas<Editor>>,
) -> Picker {
    let types = types();
    let items = types
        .iter()
        .map(|t| PickerItem::new(t.label.clone(), t.summary))
        .collect();
    let env_of = env.clone();
    Picker::new(app, env, "add", "Add a node", items, move |i, app| {
        if let Some(chosen) = types.get(i) {
            add(&env_of, editor, canvas, app, &chosen.id);
        }
    })
}

/// A place for a new card near the middle of the view, on no other card.
fn place(editor: Model<Editor>, canvas: View<NodeCanvas<Editor>>, app: &App) -> Point {
    let middle = canvas.read(app).view_center(app);
    let mut at = Point::new(
        (middle.x - NODE_WIDTH / 2.0).round(),
        (middle.y - RISE).round(),
    );
    let graph = editor.read(app);
    let taken: Vec<Point> = graph
        .node_ids()
        .into_iter()
        .map(|n| graph.position(n))
        .collect();
    while taken
        .iter()
        .any(|p| (p.x - at.x).abs() < STEP && (p.y - at.y).abs() < STEP)
    {
        at += Vec2::new(STEP, STEP);
    }
    at
}

/// Add a node of `type_id` and select it.
fn add(
    env: &Env,
    editor: Model<Editor>,
    canvas: View<NodeCanvas<Editor>>,
    app: &mut App,
    type_id: &str,
) {
    let at = place(editor, canvas, app);
    let mut added = None;
    change(editor, app, |e| {
        e.edit(|s| {
            added = Some(s.add(type_id, [f64::from(at.x), f64::from(at.y)])?);
            Ok(())
        });
    });
    let key = added.and_then(|id| {
        let e = editor.read(app);
        let index = e.session().nodes().iter().position(|n| n.id == id)?;
        e.canvas_id(index)
    });
    if let Some(key) = key {
        canvas.update(app, |c, cx| {
            c.set_selection([key]);
            cx.notify();
        });
    }
    focus_canvas(env, app);
}
