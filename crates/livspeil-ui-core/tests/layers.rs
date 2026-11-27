// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Layers (compositor properties), frame tasks and layout boundaries: a
//! property change renders no view and runs no layout, everything that
//! reads positions agrees with it, and the retained tree always equals one
//! built from scratch.

use livspeil_ui_core::{
    App, Element, ElementId, Layer, Model, PaintCmd, Render, TestHost, View, ViewContext, div,
};
use livspeil_ui_layout::{Point, Rect, Size, Transform2D, Vec2};
use livspeil_ui_theme::Color;

const SIZE: Size = Size::new(400.0, 300.0);

#[derive(Clone, Debug, PartialEq)]
struct Card {
    key: u32,
    rect: Rect,
    rows: u32,
}

/// A clipping viewport over a layer of absolute cards, with an indicator
/// leaf placed by a second layer.
#[derive(Clone, Debug, PartialEq)]
struct State {
    cards: Vec<Card>,
    cull: bool,
}

struct Board {
    state: Model<State>,
    /// A layer of another owner, attached to a leaf of its own.
    borrowed: Option<Layer>,
}

impl Render for Board {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let state = self.state.read(cx).clone();
        let (world, mark) = (cx.layer("world"), cx.layer("mark"));
        let mut content = div().name("world").w_full().h_full().layer(world);
        if state.cull {
            content = content.cull_offscreen();
        }
        for c in &state.cards {
            let mut card = div()
                .key(format!("card-{}", c.key))
                .name(format!("card-{}", c.key))
                .absolute(c.rect)
                .flex_col()
                .focusable()
                .bg(Color::WHITE);
            for r in 0..c.rows {
                card = card.child(
                    div()
                        .key(format!("row-{r}"))
                        .name(format!("row-{}-{r}", c.key))
                        .h(10.0)
                        .bg(Color::BLACK),
                );
            }
            content = content.child(card);
        }
        let borrowed = self.borrowed.map(|layer| {
            div()
                .key("borrowed")
                .name("borrowed")
                .absolute(Rect::new(0.0, 0.0, 4.0, 4.0))
                .layer(layer)
        });
        div()
            .name("viewport")
            .size(200.0, 150.0)
            .clip()
            .child(content)
            .child(
                div()
                    .key("mark")
                    .name("mark")
                    .absolute(Rect::new(0.0, 0.0, 4.0, 4.0))
                    .layer(mark)
                    .bg(Color::BLACK),
            )
            .children(borrowed)
    }
}

struct Fx {
    h: TestHost,
    view: View<Board>,
    state: Model<State>,
}

fn board(state: State) -> Fx {
    let mut app = App::new();
    let state = app.new_model(state);
    let view = app.new_view(Board {
        state,
        borrowed: None,
    });
    app.set_root(view);
    let mut h = TestHost::new(app, SIZE);
    h.run_until_idle(20);
    Fx { h, view, state }
}

fn two_cards() -> State {
    State {
        cards: vec![
            Card {
                key: 1,
                rect: Rect::new(10.0, 10.0, 40.0, 30.0),
                rows: 1,
            },
            Card {
                key: 2,
                rect: Rect::new(300.0, 10.0, 40.0, 30.0),
                rows: 1,
            },
        ],
        cull: true,
    }
}

impl Fx {
    fn layer(&mut self, key: &str) -> Layer {
        self.h.app.layer(self.view.id(), key)
    }
    fn find(&self, name: &str) -> ElementId {
        self.h.app.find(name).unwrap()
    }
    fn visual(&self, name: &str) -> Rect {
        self.h.app.visual_bounds(self.find(name)).unwrap()
    }
    fn hit(&self, x: f32, y: f32) -> Option<String> {
        let id = self.h.app.hit_test(Point::new(x, y))?;
        self.h.app.label(id)
    }
    /// Labels of the elements that painted a quad.
    fn painted(&self) -> Vec<String> {
        self.h
            .scene()
            .unwrap()
            .commands
            .iter()
            .filter_map(|c| match c {
                PaintCmd::Quad(q) => self.h.app.label(q.element),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_property_change_draws_a_frame_without_render_diff_or_layout() {
    let mut f = board(two_cards());
    assert!(!f.h.app.needs_frame());
    assert_eq!(f.hit(20.0, 15.0).as_deref(), Some("row-1-0"));
    assert_eq!(f.painted(), ["card-1", "row-1-0", "mark"]);
    let (renders, layouts) = (f.h.app.render_count(f.view.id()), f.h.app.layout_stats());
    let (card, row) = (f.find("card-2"), f.find("row-2-0"));
    f.h.app.focus(card);
    f.h.app.focus_next();
    f.h.app.focus_prev();
    f.h.run_until_idle(20);

    let world = f.layer("world");
    assert!(world.set_transform(&mut f.h.app, Transform2D::translate(Vec2::new(-200.0, 5.0))));
    assert!(f.h.app.needs_frame());
    let frame = f.h.step().unwrap();
    assert_eq!(frame.stats.views_rendered, 0);
    assert_eq!(frame.stats.reconciled, 0);
    assert!(!frame.stats.layout);
    assert!(!f.h.app.needs_frame());
    assert_eq!(f.h.app.render_count(f.view.id()), renders);
    assert_eq!(f.h.app.layout_stats(), layouts);

    // Painting (with the other card now culled), the focus ring (painted
    // inside the layer), hit testing and every position query agree.
    assert_eq!(f.painted(), ["card-2", "row-2-0", "card-2", "mark"]);
    assert_eq!(f.hit(110.0, 20.0).as_deref(), Some("row-2-0"));
    assert_eq!(f.hit(20.0, 20.0).as_deref(), Some("viewport"));
    assert_eq!(f.visual("card-2"), Rect::new(100.0, 15.0, 40.0, 30.0));
    assert_eq!(
        f.h.app.bounds(card),
        Some(Rect::new(300.0, 10.0, 40.0, 30.0)),
        "layout bounds do not move"
    );
    assert_eq!(
        f.h.app.semantic_node(card).unwrap().bounds,
        f.visual("card-2")
    );
    assert_eq!(
        f.h.app.to_layout(row, Point::new(110.0, 20.0)),
        Some(Point::new(310.0, 15.0))
    );
    let viewport = f.find("viewport");
    assert_eq!(
        f.h.app.bounds_within(row, viewport),
        Some(Rect::new(100.0, 15.0, 40.0, 10.0))
    );
    assert_eq!(
        f.h.app.bounds_within(row, f.find("world")),
        Some(Rect::new(300.0, 10.0, 40.0, 10.0))
    );
    assert_eq!(f.h.app.bounds_within(viewport, row), None);
    // Setting the same value again schedules nothing.
    assert!(world.set_transform(&mut f.h.app, Transform2D::translate(Vec2::new(-200.0, 5.0))));
    assert!(!f.h.app.needs_frame());
}

#[test]
fn moved_since_reports_what_moved() {
    let mut f = board(two_cards());
    let seen = f.h.app.revision();
    let content = f.h.app.content_revision();
    let world = f.layer("world");
    world.set_transform(&mut f.h.app, Transform2D::translate(Vec2::new(1.0, 2.0)));
    f.h.step().unwrap();
    assert_eq!(f.h.app.content_revision(), content);
    assert!(f.h.app.revision() > seen);
    let mut moved = Vec::new();
    f.h.app
        .moved_since(seen, |id, r| moved.push((f.h.app.label(id).unwrap(), r)));
    moved.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(moved.len(), 5, "{moved:?}");
    assert_eq!(
        moved[0],
        ("card-1".into(), Rect::new(11.0, 12.0, 40.0, 30.0))
    );
    let seen = f.h.app.revision();
    let mut none = 0;
    f.h.app.moved_since(seen, |_, _| none += 1);
    assert_eq!(none, 0);
    // A render is a content change.
    f.state.update(&mut f.h.app, |_, cx| cx.notify());
    f.h.step().unwrap();
    assert!(f.h.app.content_revision() > content);
}

#[test]
fn a_hidden_or_released_layer_is_safe() {
    let mut f = board(two_cards());
    let world = f.layer("world");
    assert!(world.is_alive(&f.h.app));
    assert!(world.set_opacity(&mut f.h.app, 0.0));
    f.h.step().unwrap();
    assert_eq!(f.painted(), ["mark"]);
    assert_eq!(f.hit(20.0, 20.0).as_deref(), Some("viewport"));
    assert_eq!(f.h.app.opacity_of(f.find("world")), Some(0.0));

    f.h.app.release(f.view.id());
    assert!(!world.is_alive(&f.h.app));
    assert!(!world.set_transform(&mut f.h.app, Transform2D::translate(Vec2::new(9.0, 0.0))));
    assert!(!world.set_opacity(&mut f.h.app, 0.5));
    assert!(!world.set_frame(&mut f.h.app, None));
    assert_eq!(world.transform(&f.h.app), None);
}

#[test]
fn a_frame_override_moves_and_sizes_a_leaf_without_layout() {
    let mut f = board(two_cards());
    let layouts = f.h.app.layout_stats();
    let mark = f.layer("mark");
    mark.set_frame(&mut f.h.app, Some(Rect::new(20.4, 30.5, 10.0, 8.0)));
    let frame = f.h.step().unwrap();
    assert!(!frame.stats.layout && frame.stats.views_rendered == 0);
    assert_eq!(f.h.app.layout_stats(), layouts);
    assert_eq!(f.visual("mark"), Rect::new(20.0, 31.0, 10.0, 8.0));
    assert_eq!(f.hit(25.0, 35.0).as_deref(), Some("mark"));
    // It survives a render and a layout of its parent.
    f.state.update(&mut f.h.app, |s, cx| {
        s.cards[0].rows = 2;
        cx.notify();
    });
    f.h.run_until_idle(20);
    assert_eq!(f.visual("mark"), Rect::new(20.0, 31.0, 10.0, 8.0));
    mark.set_frame(&mut f.h.app, None);
    f.h.run_until_idle(20);
    assert_eq!(f.visual("mark"), Rect::new(0.0, 0.0, 4.0, 4.0));
}

#[test]
fn hover_follows_content_moving_under_the_pointer() {
    let mut f = board(two_cards());
    let (one, two) = (f.find("card-1"), f.find("card-2"));
    f.h.pointer_move(20.0, 20.0);
    f.h.run_until_idle(20);
    assert!(f.h.app.is_hovered(one) && !f.h.app.is_hovered(two));
    let world = f.layer("world");
    world.set_transform(&mut f.h.app, Transform2D::translate(Vec2::new(-290.0, 0.0)));
    f.h.run_until_idle(20);
    assert!(!f.h.app.is_hovered(one) && f.h.app.is_hovered(two));
}

#[test]
fn moving_or_editing_a_card_lays_out_only_what_changed() {
    let mut f = board(two_cards());
    let before = f.h.app.layout_stats();
    f.state.update(&mut f.h.app, |s, cx| {
        s.cards[0].rect = Rect::new(60.0, 40.0, 40.0, 30.0);
        cx.notify();
    });
    let frame = f.h.step().unwrap();
    assert!(!frame.stats.layout, "a move runs no layout");
    assert_eq!(f.visual("row-1-0"), Rect::new(60.0, 40.0, 40.0, 10.0));
    assert_eq!(f.hit(70.0, 45.0).as_deref(), Some("row-1-0"));

    f.state.update(&mut f.h.app, |s, cx| {
        s.cards[0].rows = 3;
        cx.notify();
    });
    let frame = f.h.step().unwrap();
    assert!(frame.stats.layout);
    let after = f.h.app.layout_stats();
    assert_eq!(after.computes, before.computes, "no layout of the window");
    assert_eq!(after.boundary_computes, before.boundary_computes + 1);
    assert_eq!(f.visual("row-1-2"), Rect::new(60.0, 60.0, 40.0, 10.0));
}

#[test]
fn tasks_run_before_rendering_until_done() {
    let mut f = board(two_cards());
    let id = f.view.id();
    let world = f.layer("world");
    let state = f.state;
    let mut left = 3;
    f.h.app.animate(id, "slide", move |app, dt| {
        assert!(dt > 0.0);
        left -= 1;
        let at = Vec2::new(0.0, f32::from(3_u8 - left));
        world.set_transform(app, Transform2D::translate(at));
        if left == 0 {
            // Content that must change is rendered in the same frame.
            state.update(app, |s, cx| {
                s.cards.truncate(1);
                cx.notify();
            });
        }
        left > 0
    });
    assert!(f.h.app.is_animating(id, "slide") && f.h.app.needs_frame());
    let renders: Vec<usize> = std::iter::from_fn(|| f.h.step().map(|f| f.stats.views_rendered))
        .take(10)
        .collect();
    assert_eq!(renders, [0, 0, 1]);
    assert!(!f.h.app.is_animating(id, "slide"));
    assert_eq!(f.visual("card-1"), Rect::new(10.0, 13.0, 40.0, 30.0));
    assert!(f.h.app.find("card-2").is_none());

    // Registering again replaces; releasing the owner drops it.
    f.h.app.animate(id, "slide", |_, _| panic!("replaced"));
    f.h.app.animate(id, "slide", |_, _| true);
    assert_eq!(f.h.step().map(|f| f.stats.views_rendered), Some(0));
    f.h.app.release(id);
    assert!(!f.h.app.is_animating(id, "slide"));
}

#[test]
fn a_running_task_can_stop_and_see_the_others() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut f = board(two_cards());
    let id = f.view.id();
    let log = Rc::new(RefCell::new(Vec::new()));
    let seen = log.clone();
    f.h.app.animate(id, "a", move |app, _| {
        seen.borrow_mut()
            .push(("a", app.is_animating(id, "a"), app.is_animating(id, "b")));
        app.stop_animation(id, "b");
        false
    });
    let seen = log.clone();
    f.h.app.animate(id, "b", move |_, _| {
        seen.borrow_mut().push(("b", true, true));
        true
    });
    f.h.step().unwrap();
    assert_eq!(
        *log.borrow(),
        [("a", true, true)],
        "b was stopped before its turn"
    );
    assert!(!f.h.app.is_animating(id, "a") && !f.h.app.is_animating(id, "b"));

    // A task that stops itself stays stopped; one added while running
    // starts with the next frame.
    let runs = Rc::new(RefCell::new(Vec::new()));
    let seen = runs.clone();
    f.h.app.animate(id, "self", move |app, _| {
        app.stop_animation(id, "self");
        let seen = seen.clone();
        seen.borrow_mut().push("self");
        app.animate(id, "later", move |_, _| {
            seen.borrow_mut().push("later");
            false
        });
        true
    });
    f.h.step().unwrap();
    assert_eq!(*runs.borrow(), ["self"]);
    f.h.step().unwrap();
    assert_eq!(*runs.borrow(), ["self", "later"]);
    assert!(!f.h.app.is_animating(id, "self") && !f.h.app.needs_frame());
}

#[test]
fn releasing_the_owner_of_an_attached_layer_redraws() {
    let mut f = board(two_cards());
    // A layer owned by another entity than the view that attaches it.
    let owner = f.h.app.new_model(());
    let borrowed = f.h.app.layer(owner.id(), "x");
    f.view.update(&mut f.h.app, |v, cx| {
        v.borrowed = Some(borrowed);
        cx.notify();
    });
    borrowed.set_frame(&mut f.h.app, Some(Rect::new(20.0, 30.0, 10.0, 8.0)));
    f.h.run_until_idle(20);
    assert_eq!(f.visual("borrowed"), Rect::new(20.0, 30.0, 10.0, 8.0));
    let seen = f.h.app.revision();
    f.h.app.release(owner.id());
    assert!(f.h.app.needs_frame() && f.h.app.revision() > seen);
    f.h.run_until_idle(20);
    assert_eq!(f.visual("borrowed"), Rect::new(0.0, 0.0, 4.0, 4.0));
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }

    fn quarter(&mut self, min: i32, max: i32) -> f32 {
        let n = self.below(u64::try_from((max - min) * 4).unwrap());
        #[allow(clippy::cast_precision_loss)]
        let v = n as f32 / 4.0;
        v + min as f32
    }
}

#[derive(Clone, Copy)]
struct Props {
    world: Transform2D,
    mark: Option<Rect>,
}

/// The retained scene, hit tests and bounds equal those of a new app built
/// from the same state and properties, after random sequences of property
/// changes and edits.
#[test]
fn retained_equals_fresh() {
    for seed in 0..60 {
        let mut rng = Rng(seed);
        let mut state = two_cards();
        let mut props = Props {
            world: Transform2D::IDENTITY,
            mark: None,
        };
        let mut f = board(state.clone());
        let (world, mark) = (f.layer("world"), f.layer("mark"));
        let mut next_key = 3;
        for step in 0..60 {
            match rng.below(9) {
                0 => {
                    let zoom = 0.5 + rng.quarter(0, 2) / 2.0;
                    let pan = Vec2::new(rng.quarter(-200, 50), rng.quarter(-50, 50));
                    props.world = Transform2D::scale(zoom, zoom).then(Transform2D::translate(pan));
                    world.set_transform(&mut f.h.app, props.world);
                }
                1 => {
                    props.mark = (rng.below(3) > 0).then(|| {
                        Rect::new(
                            rng.quarter(0, 180),
                            rng.quarter(0, 130),
                            rng.quarter(1, 20),
                            rng.quarter(1, 20),
                        )
                    });
                    mark.set_frame(&mut f.h.app, props.mark);
                }
                2 => {
                    state.cards.push(Card {
                        key: next_key,
                        rect: Rect::new(
                            rng.quarter(-20, 300),
                            rng.quarter(-20, 150),
                            rng.quarter(5, 60),
                            rng.quarter(5, 60),
                        ),
                        rows: u32::try_from(rng.below(3)).unwrap(),
                    });
                    next_key += 1;
                }
                3 if !state.cards.is_empty() => {
                    let i = usize::try_from(rng.below(state.cards.len() as u64)).unwrap();
                    state.cards.remove(i);
                }
                4 | 5 if !state.cards.is_empty() => {
                    let i = usize::try_from(rng.below(state.cards.len() as u64)).unwrap();
                    state.cards[i].rect.x = rng.quarter(-20, 300);
                    state.cards[i].rect.y = rng.quarter(-20, 150);
                }
                6 if !state.cards.is_empty() => {
                    let i = usize::try_from(rng.below(state.cards.len() as u64)).unwrap();
                    state.cards[i].rows = u32::try_from(rng.below(4)).unwrap();
                    state.cards[i].rect.width = rng.quarter(5, 60);
                }
                7 if !state.cards.is_empty() => {
                    let i = usize::try_from(rng.below(state.cards.len() as u64)).unwrap();
                    let card = state.cards.remove(i);
                    state.cards.push(card);
                }
                8 => state.cull = !state.cull,
                _ => {}
            }
            let next = state.clone();
            if *f.state.read(&f.h.app) != next {
                f.state.update(&mut f.h.app, |s, cx| {
                    *s = next;
                    cx.notify();
                });
            }
            if rng.below(100) < 60 {
                f.h.run_until_idle(20);
                let mut fresh = board(state.clone());
                let (w, m) = (fresh.layer("world"), fresh.layer("mark"));
                w.set_transform(&mut fresh.h.app, props.world);
                m.set_frame(&mut fresh.h.app, props.mark);
                // Draw even if nothing differs from the defaults.
                fresh.h.app.request_frame();
                fresh.h.run_until_idle(20);
                f.h.app.request_frame();
                f.h.run_until_idle(20);
                let at = format!("seed {seed} step {step}");
                assert_eq!(f.h.scene(), fresh.h.scene(), "{at}");
                assert_eq!(f.h.app.dump_tree(), fresh.h.app.dump_tree(), "{at}");
                for _ in 0..20 {
                    let (x, y) = (rng.quarter(0, 220), rng.quarter(0, 170));
                    assert_eq!(f.hit(x, y), fresh.hit(x, y), "{at} at {x},{y}");
                }
                for c in &state.cards {
                    let name = format!("card-{}", c.key);
                    assert_eq!(f.visual(&name), fresh.visual(&name), "{at}");
                }
            }
        }
    }
}

/// The same key of the same owner is the same layer.
#[test]
fn layers_are_per_view_and_key() {
    let mut app = App::new();
    let a = app.new_model(());
    let first = app.layer(a.id(), "x");
    assert_eq!(app.layer(a.id(), "x"), first);
    assert_ne!(app.layer(a.id(), "y"), first);
    let b = app.new_model(());
    assert_ne!(app.layer(b.id(), "x"), first);
}
