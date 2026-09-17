//! `focus-nearest <dir>`: the directional search of `center-nearest` with
//! reveal semantics. A target already fully on screen only takes the keyboard
//! and the top of the z-order; a clipped one pans in exactly as
//! `center-nearest` would; a bare `navigation.anchors` point still centers.
//! `center-nearest` itself keeps parking unconditionally — it is the
//! walk-through for a non-center `focus_placement`.

use driftwm::config::{Action, Direction};
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Size};

use crate::state::{FocusIntent, StageWindow};

use super::{Fixture, client::ClientId, config, map_window, settle, window_by_app_id};

/// Park the viewport at the canvas origin, zoom 1, nothing in flight: the
/// usable area is then the canvas rect (0,0)–(1920,1080).
fn origin_view(f: &mut Fixture) {
    f.state().with_output_state(|os| {
        os.camera = Point::from((0.0, 0.0));
        os.camera_target = None;
        os.zoom = 1.0;
        os.zoom_target = None;
        os.zoom_animation_anchor = None;
    });
}

/// Map a 300×200 window and put it at `pos` on the canvas.
fn window_at(f: &mut Fixture, id: ClientId, app_id: &str, pos: (i32, i32)) -> Window {
    map_window(f, id, app_id, (300, 200));
    let window = window_by_app_id(f, app_id).unwrap();
    f.state()
        .map_window(StageWindow::Client(window.clone()), Point::from(pos), false);
    window
}

/// Three windows in a row — `a` at x=100, `b` at x=700, `c` at x=1300 — all
/// fully inside the origin view, with focus on `a` and nothing in flight.
fn row_of_three(f: &mut Fixture) -> (Window, Window, Window) {
    let id = f.add_client();
    let a = window_at(f, id, "a", (100, 400));
    let b = window_at(f, id, "b", (700, 400));
    let c = window_at(f, id, "c", (1300, 400));
    origin_view(f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);
    (a, b, c)
}

fn topmost(f: &mut Fixture) -> Option<Window> {
    f.state()
        .stage
        .windows()
        .last()
        .and_then(|w| w.client())
        .cloned()
}

/// Where `elem`'s visual frame sits on screen at the current camera and zoom:
/// (left edge, center).
fn frame_on_screen(f: &mut Fixture, elem: &StageWindow) -> (f64, Point<f64, Logical>) {
    let rect = f.state().visual_frame_rect(elem).unwrap();
    let camera = f.state().camera();
    let zoom = f.state().zoom();
    let x_low = (rect.x_low - camera.x) * zoom;
    let center = Point::from((
        ((rect.x_low + rect.x_high) / 2.0 - camera.x) * zoom,
        ((rect.y_low + rect.y_high) / 2.0 - camera.y) * zoom,
    ));
    (x_low, center)
}

/// The scenario the action exists for: every window already on screen, so a
/// directional step moves the keyboard, not the view.
#[test]
fn a_fully_visible_target_takes_focus_without_moving_the_camera() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b, _c) = row_of_three(&mut f);
    assert!(
        f.state().window_fully_in_viewport(&b),
        "precondition: b is fully on screen"
    );
    let camera = f.state().camera();

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert_eq!(
        f.state().focused_window(),
        Some(b.clone()),
        "focus stepped right to b"
    );
    assert_eq!(topmost(&mut f), Some(b), "and raised it");
    assert_eq!(f.state().camera_target(), None, "no pan was armed");
    assert_eq!(f.state().camera(), camera, "and the camera stayed put");
}

/// A target hanging off the viewport edge still pans in — a half-visible
/// window is never left where it is, matching click-to-navigate — and lands
/// on the `focus_placement` point exactly as `center-nearest` would.
#[test]
fn a_clipped_target_pans_in_onto_the_placement_point() {
    let mut f = Fixture::with_config(config(r#"focus_placement = "left""#));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (100, 400));
    // Right edge at x=2050, past the 1920 px viewport.
    let b = window_at(&mut f, id, "b", (1750, 400));
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);
    assert!(
        !f.state().window_fully_in_viewport(&b),
        "precondition: b is clipped"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert_eq!(f.state().focused_window(), Some(b.clone()));
    assert!(f.state().camera_target().is_some(), "a pan was armed");
    settle(&mut f);

    let gap = f.state().config.snap_outer_gap;
    let zoom = f.state().zoom();
    let usable = f.state().get_usable_area();
    let (x_low, _) = frame_on_screen(&mut f, &StageWindow::Client(b));
    let expected = usable.loc.x as f64 + gap * zoom;
    assert!(
        (x_low - expected).abs() < 1e-6,
        "b parks on the left placement point: {x_low} vs {expected}"
    );
}

/// `center-nearest` keeps parking a target that is already fully on screen:
/// it is the walk-through for a non-center `focus_placement`, the one
/// directional action a user can rely on to move the view.
#[test]
fn center_nearest_still_parks_a_fully_visible_target() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b, _c) = row_of_three(&mut f);

    f.state()
        .execute_action(&Action::CenterNearest(Direction::Right));

    assert_eq!(f.state().focused_window(), Some(b.clone()));
    assert!(
        f.state().camera_target().is_some(),
        "center-nearest arms a pan even for a visible target"
    );
    settle(&mut f);

    let (_, center) = frame_on_screen(&mut f, &StageWindow::Client(b));
    let vc = f.state().usable_center_screen();
    assert!(
        (center.x - vc.x).abs() < 1e-6 && (center.y - vc.y).abs() < 1e-6,
        "b settles on the viewport center: {center:?} vs {vc:?}"
    );
}

/// A bare `navigation.anchors` point is handled as `center-nearest` handles
/// it: the camera centers on the anchor and focus clears, so the next step
/// searches from the viewport center — the anchor.
#[test]
fn an_anchor_target_centers_and_clears_focus_like_center_nearest() {
    let mut f = Fixture::with_config(config("[navigation]\nanchors = [[2000, -400]]\n"));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (100, 400));
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert!(f.state().window_focus.is_none(), "focus cleared");
    let target = f
        .state()
        .camera_target()
        .expect("the camera heads for the anchor");
    let zoom = f.state().zoom();
    let vc = f.state().usable_center_screen();
    // The Y-up anchor (2000, -400) is canvas (2000, 400).
    let on_screen = ((2000.0 - target.x) * zoom, (400.0 - target.y) * zoom);
    assert!(
        (on_screen.0 - vc.x).abs() < 1e-6 && (on_screen.1 - vc.y).abs() < 1e-6,
        "the anchor lands on the viewport center: {on_screen:?} vs {vc:?}"
    );
}

/// A suspended stand-in fully on screen is focused in place, the way a live
/// window is; the camera holds.
#[test]
fn a_fully_visible_stand_in_is_focused_in_place() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (100, 400));
    origin_view(&mut f);
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((700, 400)),
        Size::from((300, 200)),
        "s",
        "S",
    );
    let s = f.state().find_suspended(sid).unwrap();
    assert!(
        f.state()
            .window_fully_in_viewport(&StageWindow::Suspended(s)),
        "precondition: the stand-in is fully on screen"
    );
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert!(
        matches!(f.state().window_focus, Some(FocusIntent::Suspended(s)) if s == sid),
        "the stand-in took the focus intent"
    );
    assert_eq!(f.state().camera_target(), None, "without a pan");

    f.state().dismiss_suspended(sid);
}

/// A clipped stand-in pans in like a clipped window.
#[test]
fn a_clipped_stand_in_pans_in() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (100, 400));
    origin_view(&mut f);
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((1750, 400)),
        Size::from((300, 200)),
        "s",
        "S",
    );
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert!(
        matches!(f.state().window_focus, Some(FocusIntent::Suspended(s)) if s == sid),
        "the stand-in took the focus intent"
    );
    assert!(f.state().camera_target().is_some(), "and a pan was armed");
    settle(&mut f);
    let s = f.state().find_suspended(sid).unwrap();
    assert!(
        f.state()
            .window_fully_in_viewport(&StageWindow::Suspended(s)),
        "the stand-in is on screen once the pan lands"
    );

    f.state().dismiss_suspended(sid);
}
