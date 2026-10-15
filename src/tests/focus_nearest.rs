//! `focus-nearest <dir>`: the directional search of `center-nearest` with
//! reveal semantics. A target already fully on screen only takes the keyboard
//! and the top of the z-order; a clipped one pans in only as far as it takes
//! to show it whole, `outer_gap` in from the edge; a bare `navigation.anchors`
//! point still centers. `center-nearest` itself keeps parking unconditionally
//! — it is the walk-through for a non-center `focus_placement`.

use driftwm::config::{Action, Direction};
use smithay::desktop::Window;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Size};

use crate::state::{FocusIntent, StageWindow};

use super::{Fixture, TICK, client::ClientId, config, map_window, settle, window_by_app_id};

const GAP_CONFIG: &str = "[snap]\nouter_gap = 20.0\n";
const GAP: f64 = 20.0;

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
    sized_window_at(f, id, app_id, pos, (300, 200))
}

fn sized_window_at(
    f: &mut Fixture,
    id: ClientId,
    app_id: &str,
    pos: (i32, i32),
    size: (u16, u16),
) -> Window {
    map_window(f, id, app_id, size);
    let window = window_by_app_id(f, app_id).unwrap();
    f.state()
        .map_window(StageWindow::Client(window.clone()), Point::from(pos), false);
    window
}

/// `a` (300×200) focused in the middle of the origin view and `b` (`size`) at
/// `b_pos`, nothing in flight.
fn focused_a_and_b(f: &mut Fixture, b_pos: (i32, i32), size: (u16, u16)) -> (Window, Window) {
    let id = f.add_client();
    let a = window_at(f, id, "a", (810, 440));
    let b = sized_window_at(f, id, "b", b_pos, size);
    origin_view(f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);
    (a, b)
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

/// A visual frame's edges in screen px.
#[derive(Debug)]
struct ScreenFrame {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl ScreenFrame {
    fn center(&self) -> Point<f64, Logical> {
        Point::from((
            (self.left + self.right) / 2.0,
            (self.top + self.bottom) / 2.0,
        ))
    }
}

/// Where `elem`'s visual frame sits on screen at the current camera and zoom.
fn frame_on_screen(f: &mut Fixture, elem: &StageWindow) -> ScreenFrame {
    let rect = f.state().visual_frame_rect(elem).unwrap();
    let camera = f.state().camera();
    let zoom = f.state().zoom();
    ScreenFrame {
        left: (rect.x_low - camera.x) * zoom,
        top: (rect.y_low - camera.y) * zoom,
        right: (rect.x_high - camera.x) * zoom,
        bottom: (rect.y_high - camera.y) * zoom,
    }
}

/// Picks one edge out of a [`ScreenFrame`].
type Edge = fn(&ScreenFrame) -> f64;

fn assert_near(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "{what}: {actual} vs {expected}"
    );
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
/// window is never left where it is, matching click-to-navigate — but only as
/// far as it takes to show it whole, `outer_gap` in from the edge it hung
/// past. The other axis holds.
#[test]
fn a_clipped_target_pans_in_just_far_enough_to_show_it() {
    // Each target sits off the view's center on the other axis, so a pan that
    // centered it there would show.
    let cases: [(Direction, (i32, i32), Edge, f64); 4] = [
        (Direction::Right, (1750, 100), |s| s.right, 1920.0 - GAP),
        (Direction::Left, (-130, 700), |s| s.left, GAP),
        (Direction::Down, (500, 980), |s| s.bottom, 1080.0 - GAP),
        (Direction::Up, (1100, -100), |s| s.top, GAP),
    ];
    for (dir, b_pos, edge, expected) in cases {
        let mut f = Fixture::with_config(config(GAP_CONFIG));
        f.add_output(1, (1920, 1080));
        f.skip_baseline_check();
        let (_a, b) = focused_a_and_b(&mut f, b_pos, (300, 200));
        assert!(
            !f.state().window_fully_in_viewport(&b),
            "precondition ({dir:?}): b is clipped"
        );
        let camera = f.state().camera();

        f.state().execute_action(&Action::FocusNearest(dir.clone()));

        assert_eq!(f.state().focused_window(), Some(b.clone()), "{dir:?}");
        assert!(
            f.state().camera_target().is_some(),
            "{dir:?}: a pan was armed"
        );
        settle(&mut f);
        let frame = frame_on_screen(&mut f, &StageWindow::Client(b));
        assert_near(
            edge(&frame),
            expected,
            &format!("{dir:?}: b's leading edge"),
        );
        let moved = f.state().camera() - camera;
        let cross_axis = match dir {
            Direction::Left | Direction::Right => moved.y,
            _ => moved.x,
        };
        assert_eq!(cross_axis, 0.0, "{dir:?}: the other axis held");
    }
}

/// `focus_placement` is where jumps land, not walks: a directional step onto
/// a clipped window under `"left"` still stops as soon as the window is whole
/// rather than carrying it across to the left edge.
#[test]
fn focus_placement_does_not_move_where_focus_nearest_lands() {
    let mut f = Fixture::with_config(config(&format!("focus_placement = \"left\"\n{GAP_CONFIG}")));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b) = focused_a_and_b(&mut f, (1750, 440), (300, 200));

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);

    let frame = frame_on_screen(&mut f, &StageWindow::Client(b));
    assert_near(frame.right, 1920.0 - GAP, "b's right edge");
}

/// On an axis where the frame and its two gaps don't fit, the step centers
/// the target, as a park does on that axis; the other axis still pans
/// minimally.
#[test]
fn a_target_taller_than_the_view_centers_vertically_and_pans_minimally_across() {
    let mut f = Fixture::with_config(config(GAP_CONFIG));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b) = focused_a_and_b(&mut f, (1750, -500), (300, 1200));

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);

    let frame = frame_on_screen(&mut f, &StageWindow::Client(b));
    assert_near(
        frame.bottom - frame.top,
        1200.0,
        "precondition: b is 1200 tall",
    );
    assert_near(frame.right, 1920.0 - GAP, "b's right edge");
    assert_near(frame.center().y, 540.0, "b's vertical center");
}

/// The gap is canvas px, like the placement inset: zoomed out to 0.5 it
/// shows half as wide, and the step leaves the zoom alone.
#[test]
fn the_gap_scales_with_zoom() {
    let mut f = Fixture::with_config(config(GAP_CONFIG));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    // At zoom 0.5 the view spans canvas (0,0)–(3840,2160); b's right edge at
    // canvas 4000 hangs past it.
    let (_a, b) = focused_a_and_b(&mut f, (3700, 980), (300, 200));
    f.state().with_output_state(|os| os.zoom = 0.5);
    assert!(
        !f.state().window_fully_in_viewport(&b),
        "precondition: b is clipped"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);

    assert_eq!(f.state().zoom(), 0.5, "the zoom held");
    let frame = frame_on_screen(&mut f, &StageWindow::Client(b));
    assert_near(frame.right, 1920.0 - GAP * 0.5, "b's right edge");
}

/// `outer_gap` measures to the content edge with the border outside it, so a
/// border wider than the gap would put the frame's inset past the usable
/// edge. The step clamps it there, so the frame still lands whole.
#[test]
fn a_border_wider_than_the_gap_lands_the_frame_on_the_edge() {
    let mut f = Fixture::with_config(config(
        "[snap]\nouter_gap = 2.0\n[decorations]\nborder_width = 6\n",
    ));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b) = focused_a_and_b(&mut f, (1750, 440), (300, 200));
    let elem = StageWindow::Client(b.clone());
    assert_eq!(
        f.state().element_border_width(&elem),
        6,
        "precondition: b wears the border"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);

    let frame = frame_on_screen(&mut f, &elem);
    assert_near(frame.right, 1920.0, "b's right frame edge");
    assert!(f.state().window_fully_in_viewport(&b));
}

/// `center-nearest` parks a target that is already fully on screen.
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

    let center = frame_on_screen(&mut f, &StageWindow::Client(b)).center();
    let vc = f.state().usable_center_screen();
    assert!(
        (center.x - vc.x).abs() < 1e-6 && (center.y - vc.y).abs() < 1e-6,
        "b settles on the viewport center: {center:?} vs {vc:?}"
    );
}

/// `center-nearest` onto a clipped target parks it on `focus_placement`, not
/// the minimal pan `focus-nearest` takes.
#[test]
fn center_nearest_still_parks_a_clipped_target_on_the_placement_point() {
    let mut f = Fixture::with_config(config(&format!("focus_placement = \"left\"\n{GAP_CONFIG}")));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b) = focused_a_and_b(&mut f, (1750, 440), (300, 200));

    f.state()
        .execute_action(&Action::CenterNearest(Direction::Right));
    settle(&mut f);

    let frame = frame_on_screen(&mut f, &StageWindow::Client(b));
    assert_near(frame.left, GAP, "b's left edge");
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

/// A clipped stand-in pans in like a clipped window: just far enough to show
/// its frame, `outer_gap` in from the edge. Its content is on screen and only
/// its title bar hangs off the top, so the pan has to count the bar.
#[test]
fn a_clipped_stand_in_pans_in_just_far_enough() {
    let mut f = Fixture::with_config(config(GAP_CONFIG));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (810, 440));
    origin_view(&mut f);
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((810, 0)),
        Size::from((300, 200)),
        "s",
        "S",
    );
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Up));

    assert!(
        matches!(f.state().window_focus, Some(FocusIntent::Suspended(s)) if s == sid),
        "the stand-in took the focus intent"
    );
    assert!(f.state().camera_target().is_some(), "and a pan was armed");
    settle(&mut f);
    let s = StageWindow::Suspended(f.state().find_suspended(sid).unwrap());
    let frame = frame_on_screen(&mut f, &s);
    assert_near(frame.top, GAP, "the stand-in's bar");
    assert!(
        f.state().window_fully_in_viewport(&s),
        "the stand-in is on screen once the pan lands"
    );

    f.state().dismiss_suspended(sid);
}

/// A step taken while the previous step's pan is still in flight re-targets
/// the pan rather than focusing in place, so its target is never left behind
/// off screen while the camera carries on to the previous one.
#[test]
fn a_step_mid_pan_retargets_the_pan_instead_of_focusing_in_place() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (1300, 400));
    // Far enough right that revealing it carries a off screen.
    let b = window_at(&mut f, id, "b", (4000, 400));
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    let toward_b = f
        .state()
        .camera_target()
        .expect("the pan toward b is armed");
    // The first frame of the flight: the zoom tick that drives a navigation
    // pan clears `camera_target`, so it alone no longer says "in flight".
    f.state().apply_zoom_animation(TICK);
    assert_eq!(f.state().camera_target(), None);
    assert!(f.state().zoom_target().is_some(), "still in flight");
    assert!(
        f.state().window_fully_in_viewport(&a),
        "precondition: at this frame a still reads as fully on screen"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Left));

    assert_eq!(f.state().focused_window(), Some(a.clone()));
    let toward_a = f.state().camera_target().expect("the pan is still armed");
    assert_ne!(toward_a, toward_b, "and now heads for a instead of b");
    settle(&mut f);
    assert!(
        f.state().window_fully_in_viewport(&a),
        "a is on screen once the view rests"
    );
    assert!(!f.state().window_fully_in_viewport(&b));
}

/// A step out of fullscreen searches from the window that just left it, even
/// when the restored view clips that window.
#[test]
fn a_step_out_of_fullscreen_searches_from_the_window_that_left_it() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    // From the viewport center (960, 540), c is the nearest window to the
    // right; from a, only b is.
    let c = window_at(&mut f, id, "c", (1000, 400));
    let a = window_at(&mut f, id, "a", (1750, 400));
    let b = window_at(&mut f, id, "b", (2200, 400));
    origin_view(&mut f);
    assert!(
        !f.state().window_fully_in_viewport(&a),
        "precondition: a is clipped in the view fullscreen restores to"
    );
    f.state().enter_fullscreen(&a, Some(output));
    f.double_roundtrip(id);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert!(!f.state().is_fullscreen(), "the step left fullscreen first");
    assert_eq!(
        f.state().focused_window(),
        Some(b),
        "the search started from a, not from the viewport center (which would \
         have picked {c:?})"
    );
}

/// A window too tall for the view never reads as fully on screen. Once a step
/// has shown it as fully as it fits, the next step walks on past it rather
/// than finding it again from the viewport center.
#[test]
fn a_walk_steps_on_past_a_window_too_tall_to_show_whole() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (810, 440));
    let t = sized_window_at(&mut f, id, "t", (1750, -500), (300, 1200));
    let c = window_at(&mut f, id, "c", (2300, 440));
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(t));

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(c.clone()));
    assert!(f.state().window_fully_in_viewport(&c));
}

/// At a fractional zoom, a frame landed flush on the usable edge carries
/// float rounding from the pan. It still reads as fully on screen, and the
/// next step walks on from it.
#[test]
fn a_frame_revealed_flush_at_a_fractional_zoom_reads_as_on_screen() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (-2500, 400));
    let b = sized_window_at(&mut f, id, "b", (-1498, 400), (1000, 400));
    let c = window_at(&mut f, id, "c", (0, 400));
    f.state().with_output_state(|os| {
        os.camera = Point::from((-2996.73, 0.0));
        os.camera_target = None;
        os.zoom = 0.961;
        os.zoom_target = None;
        os.zoom_animation_anchor = None;
    });
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(b.clone()));
    let frame = frame_on_screen(&mut f, &StageWindow::Client(b.clone()));
    assert!(
        frame.right > 1920.0 && frame.right - 1920.0 < 1e-9,
        "precondition: b landed a hair past the edge, {}",
        frame.right
    );
    assert!(f.state().window_fully_in_viewport(&b));

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(c));
}

/// A step during a zoom animation onto a window still on screen where the
/// zoom lands takes focus and lets the zoom finish, rather than stopping it
/// partway.
#[test]
fn a_step_during_a_zoom_animation_lets_the_zoom_land() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b, _c) = row_of_three(&mut f);
    f.state().zoom_to_anchored(0.5);
    f.state().apply_zoom_animation(TICK);
    f.state().apply_zoom_animation(TICK);
    assert!(
        f.state().zoom() > 0.5,
        "precondition: the zoom is under way"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert_eq!(f.state().focused_window(), Some(b.clone()));
    settle(&mut f);
    assert_eq!(f.state().zoom(), 0.5, "the zoom landed");
    assert!(f.state().window_fully_in_viewport(&b));
}

/// Whether a too-big window counts as shown is judged on the frame the reveal
/// moves: a window whose committed size lags the size it was configured to
/// still counts once revealed, so the next step walks on past it.
#[test]
fn a_walk_steps_on_past_a_too_tall_window_whose_size_lags_its_configure() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (810, 440));
    let t_surface = map_window(&mut f, id, "t", (300, 1200));
    let t = window_by_app_id(&mut f, "t").unwrap();
    f.state().map_window(
        StageWindow::Client(t.clone()),
        Point::from((1750, -500)),
        false,
    );
    let c = window_at(&mut f, id, "c", (2300, 440));
    let toplevel = t.toplevel().unwrap();
    toplevel.with_pending_state(|s| s.size = Some((300, 1260).into()));
    toplevel.send_configure();
    super::ack_but_keep_size(&mut f, id, &t_surface);
    assert_ne!(
        crate::state::configured_window_size(&t),
        t.geometry().size,
        "precondition: t's size lags its configure"
    );
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(t));

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    settle(&mut f);
    assert_eq!(f.state().focused_window(), Some(c));
}

/// A second step taken while the first one's pan is still under way searches
/// from where that pan lands, so it walks on rather than finding the window
/// the pan is revealing again.
#[test]
fn a_quick_second_step_walks_on_from_where_the_first_pan_lands() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    let a = window_at(&mut f, id, "a", (810, 440));
    let _b = window_at(&mut f, id, "b", (1750, 440));
    let c = window_at(&mut f, id, "c", (2300, 440));
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&a, serial);

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));
    f.state().apply_zoom_animation(TICK);
    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert_eq!(f.state().focused_window(), Some(c.clone()));
    settle(&mut f);
    assert!(f.state().window_fully_in_viewport(&c));
}

/// A step during a zoom-in onto a window the landing zoom clips pans from
/// where the zoom lands, and lets it land.
#[test]
fn a_step_during_a_zoom_in_pans_from_where_the_zoom_lands() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let (_a, b, c) = row_of_three(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&b, serial);
    // Zoomed to 2 on the viewport center the view spans canvas x 480–1440:
    // b stays whole, c at 1300–1600 is cut off.
    f.state().zoom_to_anchored(2.0);
    f.state().apply_zoom_animation(TICK);
    assert!(
        f.state().zoom() < 2.0,
        "precondition: the zoom is under way"
    );

    f.state()
        .execute_action(&Action::FocusNearest(Direction::Right));

    assert_eq!(f.state().focused_window(), Some(c.clone()));
    settle(&mut f);
    assert_eq!(f.state().zoom(), 2.0, "the zoom landed");
    assert!(f.state().window_fully_in_viewport(&c));
}
