// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Toasts: timers, hover pause, actions, stacking, motion, semantics.

mod common;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use common::{center, find, golden_dir, host_sized, scene, wait};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{Role, TestHost, View, div};
use livspeil_ui_layout::Size;
use livspeil_ui_theme::{MotionConfig, Scheme, Theme};
use livspeil_ui_widgets::{
    Env, MAX_TOASTS, TOAST_DURATION, TOAST_MARGIN, Toast, ToastTone, Toasts, toast_element,
};

struct Fx {
    h: TestHost,
    env: Env,
    toasts: View<Toasts>,
}

fn fixture(scheme: Scheme) -> Fx {
    let mut out = None;
    let (h, env) = host_sized(Size::new(520.0, 380.0), |app, env| {
        app.set_theme(Theme {
            scheme,
            ..Theme::default()
        });
        out = Some(Toasts::new(env, app));
        Box::new(|_| div().w_full().h_full())
    });
    Fx {
        h,
        env,
        toasts: out.unwrap(),
    }
}

impl Fx {
    fn show(&mut self, t: Toast) -> u64 {
        let id = Toasts::show(self.toasts, &mut self.h.app, t);
        self.h.step();
        id
    }
    fn active(&self) -> Vec<u64> {
        self.toasts.read(&self.h.app).active()
    }
    fn element(&self, id: u64) -> Option<livspeil_ui_core::ElementId> {
        let e = toast_element(&self.env, &self.h.app, id)?;
        self.h.app.bounds(e).map(|_| e)
    }
}

#[test]
fn a_toast_slides_and_fades_in_at_the_bottom_right_and_never_scales() {
    let mut f = fixture(Scheme::Dark);
    assert!(f.h.app.find("toast").is_none());
    f.h.run_until_idle(10);
    assert!(!f.h.app.needs_frame(), "idle with no toasts");
    let id = f.show(Toast::new("Saved \u{201c}Living room\u{201d}"));
    f.h.step();
    let el = f.element(id).expect("mounted");
    assert!(f.h.app.opacity_of(el).unwrap() < 1.0, "starts transparent");
    assert!(
        f.h.app.translation(el).unwrap().x > 0.0,
        "starts off to the right"
    );
    assert_eq!(f.h.app.scale_of(el), Some(1.0));
    wait(&mut f.h, 400);
    assert_eq!(f.h.app.opacity_of(el), Some(1.0));
    assert_eq!(f.h.app.translation(el).unwrap().x, 0.0);
    assert_eq!(f.h.app.scale_of(el), Some(1.0));
    let b = f.h.app.bounds(el).unwrap();
    assert_eq!(b.right(), 520.0 - TOAST_MARGIN);
    assert_eq!(b.bottom(), 380.0 - TOAST_MARGIN);
}

#[test]
fn it_dismisses_itself_after_its_duration_and_the_stack_goes_idle() {
    let mut f = fixture(Scheme::Dark);
    let id = f.show(Toast::new("Done"));
    wait(&mut f.h, 3_700);
    assert_eq!(f.active(), [id], "still there before {TOAST_DURATION:?}");
    wait(&mut f.h, 600);
    assert!(f.active().is_empty());
    f.h.run_until_idle(300);
    assert!(f.h.app.find("toast").is_none(), "removed after the exit");
    assert!(!f.h.app.needs_frame(), "idle again");
    // Custom duration and sticky toasts.
    let a = f.show(Toast::new("Quick").duration(Duration::from_millis(500)));
    let b = f.show(Toast::new("Stays").sticky());
    wait(&mut f.h, 900);
    assert_eq!(f.active(), [b]);
    assert!(f.toasts.read(&f.h.app).remaining(a).is_none());
    wait(&mut f.h, 20_000);
    assert_eq!(f.active(), [b], "sticky toasts stay");
}

#[test]
fn hover_and_focus_pause_the_countdown() {
    let mut f = fixture(Scheme::Dark);
    let id = f.show(Toast::new("Hover me").action("Undo", |_| {}));
    wait(&mut f.h, 400);
    let p = f.h.app.bounds(f.element(id).unwrap()).unwrap();
    // Over the text, not a button.
    f.h.pointer_move(p.x + 24.0, p.y + 8.0);
    wait(&mut f.h, 500);
    let left = f.toasts.read(&f.h.app).remaining(id).unwrap();
    wait(&mut f.h, 10_000);
    assert_eq!(f.active(), [id], "hover keeps it");
    let after = f.toasts.read(&f.h.app).remaining(id).unwrap();
    assert!((left - after).abs() < 0.05, "paused: {left} vs {after}");
    // Leave: the countdown resumes where it was.
    f.h.pointer_move(5.0, 5.0);
    wait(&mut f.h, 1_000);
    let resumed = f.toasts.read(&f.h.app).remaining(id).unwrap();
    assert!(resumed < after - 0.8, "{after} -> {resumed}");
    wait(&mut f.h, 4_000);
    assert!(f.active().is_empty());
    // Focus on the action button pauses it too.
    let id = f.show(Toast::new("Focus me").action("Undo", |_| {}));
    wait(&mut f.h, 300);
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "toast-action:2")));
    wait(&mut f.h, 10_000);
    assert_eq!(f.active(), [id]);
}

#[test]
fn action_close_button_and_escape_dismiss() {
    let mut f = fixture(Scheme::Dark);
    let undone = Rc::new(Cell::new(0));
    let u = undone.clone();
    let a = f.show(Toast::new("Moved 3 Charms").action("Undo", move |_| u.set(u.get() + 1)));
    wait(&mut f.h, 400);
    let p = center(&f.h, "toast-action:1");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(100);
    assert_eq!(undone.get(), 1);
    assert!(!f.active().contains(&a), "the action dismisses");
    wait(&mut f.h, 400);
    // The close button.
    let b = f.show(Toast::new("Closable"));
    wait(&mut f.h, 400);
    let p = center(&f.h, "toast-close:2");
    f.h.click(p.x, p.y);
    assert!(!f.active().contains(&b));
    wait(&mut f.h, 400);
    // Escape with focus in the toast.
    let c = f.show(Toast::new("Escape me").sticky());
    wait(&mut f.h, 400);
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "toast-close:3")));
    f.h.key("escape");
    assert!(!f.active().contains(&c));
    wait(&mut f.h, 400);
    assert!(f.h.app.find("toast").is_none());
}

#[test]
fn toasts_stack_upwards_reflow_and_are_capped() {
    let mut f = fixture(Scheme::Dark);
    let ids: Vec<u64> = (0..3)
        .map(|i| f.show(Toast::new(format!("Toast {i}")).sticky()))
        .collect();
    wait(&mut f.h, 500);
    let screen_y = |f: &Fx, id| {
        let e = f.element(id).unwrap();
        f.h.app.bounds(e).unwrap().y + f.h.app.translation(e).unwrap().y
    };
    let ys: Vec<f32> = ids.iter().map(|i| screen_y(&f, *i)).collect();
    assert!(ys[0] < ys[1] && ys[1] < ys[2], "newest is lowest: {ys:?}");
    // Dismissing the middle one moves the oldest down (with the spring).
    Toasts::dismiss(f.toasts, &mut f.h.app, ids[1]);
    wait(&mut f.h, 600);
    let now = screen_y(&f, ids[0]);
    assert!(now > ys[0], "older toast moved down: {} -> {now}", ys[0]);
    assert_eq!(f.active(), [ids[0], ids[2]]);
    // The cap: the oldest leave when more arrive.
    for i in 0..(MAX_TOASTS + 2) {
        f.show(Toast::new(format!("More {i}")).sticky());
    }
    assert_eq!(f.active().len(), MAX_TOASTS);
    wait(&mut f.h, 600);
    assert!(f.h.app.find("toast").is_some());
    Toasts::clear(f.toasts, &mut f.h.app);
    wait(&mut f.h, 600);
    assert!(f.h.app.find("toast").is_none());
    assert!(!f.h.app.needs_frame());
}

#[test]
fn reduce_motion_drops_the_slide_and_keeps_the_fade() {
    let mut f = fixture(Scheme::Dark);
    f.h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    let id = f.show(Toast::new("Calm"));
    f.h.step();
    let el = f.element(id).unwrap();
    assert_eq!(f.h.app.translation(el).unwrap().x, 0.0, "no slide");
    f.h.step();
    wait(&mut f.h, 400);
    assert_eq!(f.h.app.opacity_of(el), Some(1.0));
    // A speed of 0 removes toasts without an exit animation.
    f.h.app.set_motion(MotionConfig {
        speed: 0.0,
        ..MotionConfig::default()
    });
    Toasts::dismiss(f.toasts, &mut f.h.app, id);
    f.h.step();
    f.h.step();
    assert!(f.h.app.find("toast").is_none());
}

#[test]
fn toasts_are_live_regions() {
    let mut f = fixture(Scheme::Dark);
    let a = f.show(Toast::new("Saved").title("Project"));
    let b = f.show(Toast::new("Could not save").tone(ToastTone::Danger));
    wait(&mut f.h, 400);
    let na = f.h.app.semantic_node(f.element(a).unwrap()).unwrap();
    assert_eq!(na.role(), Some(Role::Status));
    assert_eq!(na.label(), Some("Project"));
    assert_eq!(na.semantics.unwrap().get_description(), Some("Saved"));
    let nb = f.h.app.semantic_node(f.element(b).unwrap()).unwrap();
    assert_eq!(nb.role(), Some(Role::Alert));
    assert_eq!(nb.label(), Some("Could not save"));
    let region = f.h.app.semantic_node(find(&f.h, "toast-stack")).unwrap();
    assert_eq!(region.label(), Some("Notifications"));
}

#[test]
fn toast_scenes_in_both_themes() {
    for (scheme, name) in [(Scheme::Dark, "toast-dark"), (Scheme::Light, "toast-light")] {
        let mut f = fixture(scheme);
        f.show(
            Toast::new("Moved 3 Charms to Kitchen")
                .title("Moved")
                .action("Undo", |_| {})
                .sticky(),
        );
        f.show(
            Toast::new("Living room saved")
                .tone(ToastTone::Success)
                .sticky(),
        );
        f.show(
            Toast::new("The scene could not be saved because the disk is full. Free some space and try again.")
                .title("Save failed")
                .tone(ToastTone::Danger)
                .sticky(),
        );
        wait(&mut f.h, 600);
        assert_golden(&golden_dir(), name, &scene(&f.h));
    }
}
