//! Keyboard snapping: `move-to-snap` walks the focused element to the stops
//! drag snapping offers, `resize-to-snap` walks its right or bottom edge
//! between them and the usable area's edge — clients and stand-ins alike —
//! and neither ever lands the element on another window. A move also steps
//! out a gap off an adjacent window's side.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Size};
use wayland_client::protocol::wl_surface::WlSurface as ClientSurface;

use super::client::ClientId;
use super::{
    Fixture, TICK, ack_with_size, config, configure_count, give_ssd, last_configured, map_window,
    settle, window_by_app_id, window_position,
};
use crate::state::StageWindow;
use driftwm::config::{Action, Direction};

/// Park the viewport at `camera` and zoom 1 with nothing in flight, so a
/// camera target afterwards is one an action armed.
fn park_view(f: &mut Fixture, camera: Point<f64, Logical>) {
    f.state().with_output_state(|os| {
        os.camera = camera;
        os.camera_target = None;
        os.zoom = 1.0;
        os.zoom_target = None;
        os.zoom_animation_anchor = None;
    });
}

/// Map a client window of `size` and park it at `loc`.
fn window_at(
    f: &mut Fixture,
    id: ClientId,
    app_id: &str,
    loc: (i32, i32),
    size: (u16, u16),
) -> (Window, ClientSurface) {
    let surface = map_window(f, id, app_id, size);
    let window = window_by_app_id(f, app_id).unwrap();
    f.state()
        .map_window(StageWindow::Client(window.clone()), Point::from(loc), true);
    (window, surface)
}

fn focus(f: &mut Fixture, window: &Window) {
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(window, serial);
}

fn act(f: &mut Fixture, action: Action) {
    f.state().execute_action(&action);
}

#[test]
fn move_to_snap_docks_at_the_gap_and_stops_when_nothing_is_further() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((588, 100)),
        "docked snap.gap short of the neighbour's left edge, y untouched"
    );
    assert!(
        f.state().camera_target().is_none(),
        "still fully in view, so the camera stays"
    );

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((1412, 100)),
        "the far dock, snap.gap past the neighbour's right edge"
    );

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((1412, 100)),
        "no stop further right: a no-op"
    );
    assert!(f.state().camera_target().is_none());
}

/// The alignments are reachable with `[snap] corners` and `centers` off: those
/// knobs tune drag magnetism, not what the keyboard can reach.
#[test]
fn move_to_snap_aligns_with_an_adjacent_neighbour_whatever_the_magnetism_knobs() {
    let mut f = Fixture::with_config(config("[snap]\ncorners = false\ncenters = false\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    // 12 px below `a`, inside the adjacency band, spanning x 300..900.
    window_at(&mut f, id, "b", (300, 412), (600, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    for x in [300, 400, 500] {
        act(&mut f, Action::MoveToSnap(Direction::Right));
        assert_eq!(window_position(&mut f, &a), Point::from((x, 100)));
    }
    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((912, 100)),
        "past the right-edge alignment, on to snap.gap off b's right edge"
    );
}

/// A stop that would land the window on another one is passed over: in a
/// docked row, the docks either side of the middle window are the outer
/// windows' own slots.
#[test]
fn move_to_snap_skips_stops_that_land_on_a_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "n", (512, 100), (400, 300));
    window_at(&mut f, id, "m", (924, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((1336, 100)),
        "past n's and m's slots to the dock beyond m"
    );

    act(&mut f, Action::MoveToSnap(Direction::Left));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((100, 100)),
        "and back past both to the dock before n"
    );
}

/// An alignment can land on a window too: `w`, wide below both, offers its
/// left edge, which is where `x` already sits.
#[test]
fn move_to_snap_skips_an_alignment_that_lands_on_a_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    window_at(&mut f, id, "x", (500, 100), (400, 300));
    let (a, _) = window_at(&mut f, id, "a", (912, 100), (400, 300));
    window_at(&mut f, id, "w", (500, 412), (1224, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Left));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((88, 100)),
        "past w's left edge, which is x's slot, to the dock before x"
    );
}

/// A window directly below another, already aligned with it every way, steps
/// out sideways to a gap off its side.
#[test]
fn move_to_snap_steps_out_beside_a_window_above() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    window_at(&mut f, id, "a", (600, 100), (400, 300));
    let (b, _) = window_at(&mut f, id, "b", (600, 412), (400, 300));
    focus(&mut f, &b);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Left));
    assert_eq!(
        window_position(&mut f, &b),
        Point::from((188, 412)),
        "right edge snap.gap off a's left edge, y untouched"
    );

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &b),
        Point::from((600, 412)),
        "back under a"
    );

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &b),
        Point::from((1012, 412)),
        "left edge snap.gap off a's right edge, y untouched"
    );
}

/// Stepped out beside the window above, `up` walks through its alignments
/// rather than jumping over it.
#[test]
fn move_to_snap_up_from_beside_a_window_above_walks_up_beside_it() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (600, 100), (400, 300));
    let (b, _) = window_at(&mut f, id, "b", (600, 412), (400, 200));
    focus(&mut f, &b);
    park_view(&mut f, Point::from((0.0, 0.0)));
    act(&mut f, Action::MoveToSnap(Direction::Left));

    for (y, aligned) in [(200, "bottom edges"), (150, "centres"), (100, "top edges")] {
        act(&mut f, Action::MoveToSnap(Direction::Up));
        assert_eq!(
            window_position(&mut f, &b),
            Point::from((188, y)),
            "{aligned} aligned with a's"
        );
        let a_frame = f.state().visual_frame_rect(&StageWindow::Client(a.clone()));
        let b_frame = f.state().visual_frame_rect(&StageWindow::Client(b.clone()));
        assert!(!b_frame.unwrap().overlaps(&a_frame.unwrap()), "beside a");
    }
}

/// A side dock is passed over like any stop that would land the window on a
/// third one.
#[test]
fn move_to_snap_skips_a_side_dock_that_lands_on_a_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    window_at(&mut f, id, "a", (1000, 100), (400, 300));
    let (b, _) = window_at(&mut f, id, "b", (1000, 412), (400, 300));
    // Docked left of `b`, in the slot a's side dock offers.
    window_at(&mut f, id, "c", (588, 412), (400, 300));
    focus(&mut f, &b);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Left));
    assert_eq!(
        window_position(&mut f, &b),
        Point::from((176, 412)),
        "past c's slot to the dock before c"
    );
}

/// A press while the camera is still flying is judged against where the
/// camera is going, not the frame it has reached: the pan already armed
/// would otherwise carry on to the old stop and leave the window behind.
#[test]
fn move_to_snap_mid_pan_brings_the_camera_back_with_the_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (1500, 100), (400, 300));
    window_at(&mut f, id, "c", (800, 100), (400, 300));
    window_at(&mut f, id, "b", (5000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(window_position(&mut f, &a), Point::from((4588, 100)));

    f.state().apply_zoom_animation(TICK);
    f.state().apply_camera_animation(TICK);
    assert!(
        f.state().camera_target().is_none() && f.state().zoom_target().is_some(),
        "one frame in, the anchored pan has already dropped its camera target"
    );
    let camera = f.state().camera();
    assert!(
        camera.x < 1212.0 && camera.x + 1920.0 > 1612.0,
        "precondition: the stop back left is in view in this frame: {camera:?}"
    );

    act(&mut f, Action::MoveToSnap(Direction::Left));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((1212, 100)),
        "the dock after c"
    );
    settle(&mut f);
    assert!(
        f.state().window_fully_in_viewport(&a),
        "the camera came back with the window"
    );
}

/// A press during a zoom animation follows the window at the zoom the view is
/// heading to, so the zoom lands instead of freezing where this frame caught it.
#[test]
fn move_to_snap_during_a_zoom_out_lets_the_zoom_land() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (5000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let resting = f.state().zoom_target();

    act(&mut f, Action::ZoomOut);
    let heading = f.state().zoom_target().expect("zoom-out armed a zoom");
    assert_ne!(Some(heading), resting, "the zoom target moved");
    f.state().apply_zoom_animation(TICK);
    f.state().apply_camera_animation(TICK);
    assert!(
        f.state().zoom() > heading,
        "precondition: the zoom is under way"
    );

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(window_position(&mut f, &a), Point::from((4588, 100)));
    settle(&mut f);
    assert_eq!(f.state().zoom(), heading, "the zoom-out landed");
    assert!(
        f.state().window_fully_in_viewport(&a),
        "the camera followed the window"
    );
}

/// A press while the view is flying leaves the flight alone when the stop is
/// on screen where it lands, the way a press at rest leaves a visible stop
/// alone: how quickly the keys follow each other doesn't change the view.
#[test]
fn move_to_snap_during_a_zoom_out_leaves_a_stop_the_view_lands_on_alone() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ZoomOut);
    let landing = f.state().landing_view();
    f.state().apply_zoom_animation(TICK);
    f.state().apply_camera_animation(TICK);

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(window_position(&mut f, &a), Point::from((588, 100)));
    settle(&mut f);
    assert_eq!(
        (f.state().camera(), f.state().zoom()),
        landing,
        "the zoom-out landed where it was heading"
    );
}

/// The camera follows through the directional-focus navigation, so
/// `focus_placement` decides where the window lands on screen.
#[test]
fn move_to_snap_reveals_a_far_stop_on_the_focus_placement_point() {
    let mut f = Fixture::with_config(config("focus_placement = \"left\"\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, _) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (5000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let resting = f.state().camera();

    act(&mut f, Action::MoveToSnap(Direction::Right));

    assert_eq!(
        window_position(&mut f, &a),
        Point::from((4588, 100)),
        "a neighbour screens away is a valid target"
    );
    let target = f
        .state()
        .camera_target()
        .expect("the window left the viewport, so the camera follows");
    assert_ne!(target, resting);

    settle(&mut f);
    let rect = f
        .state()
        .visual_frame_rect(&StageWindow::Client(a.clone()))
        .unwrap();
    let camera = f.state().camera();
    let usable = f.state().get_usable_area();
    let x_low = (rect.x_low - camera.x) * f.state().zoom();
    assert!(
        (x_low - usable.loc.x as f64).abs() < 1e-6,
        "focus_placement = left parks the frame's left edge on the usable area's left: {x_low}"
    );
}

#[test]
fn resize_to_snap_walks_the_right_edge_out_to_a_dock_and_back() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    // Below `a`, offering its right edge at 700 as an alignment.
    window_at(&mut f, id, "c", (100, 412), (600, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let before = window_position(&mut f, &a);

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (600, 300),
        "the right edge aligns with c's"
    );
    assert_eq!(window_position(&mut f, &a), before, "the left edge holds");

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (888, 300),
        "then stops snap.gap short of b"
    );

    let configures = configure_count(&mut f, id, &surface);
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        configure_count(&mut f, id, &surface),
        configures,
        "docked: growing on would cover b, so no configure at all"
    );

    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (600, 300),
        "back to the previous stop"
    );

    let configures = configure_count(&mut f, id, &surface);
    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(
        configure_count(&mut f, id, &surface),
        configures,
        "no stop inside the window: a no-op"
    );
    assert!(
        f.state().camera_target().is_none(),
        "a resize never moves the camera"
    );
}

/// The fill edge: the usable area's edge, `snap.outer_gap` in.
#[test]
fn resize_to_snap_with_no_neighbour_grows_to_the_fill_edge() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 12.0\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (1808, 300));
    act(&mut f, Action::ResizeToSnap(Direction::Down));
    assert_eq!(last_configured(&mut f, id, &surface), (1808, 968));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((100, 100)),
        "both edges grew away from the top-left"
    );
}

/// Only a window the frame would newly cover is in the way: one it already
/// overlaps doesn't stop it short of the fill edge.
#[test]
fn resize_to_snap_grows_over_a_window_it_already_overlaps() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 12.0\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (400, 200), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (1808, 300));
}

/// A cell-rounding client answers short of the stop or past it; the walk goes
/// on from the stop it asked for, else every press re-asks for the same size.
#[test]
fn resize_to_snap_walks_on_past_a_cell_rounded_answer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    // Above and below `a`, offering their right edges at 350 and 700.
    window_at(&mut f, id, "d", (100, -212), (250, 300));
    window_at(&mut f, id, "c", (100, 412), (600, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (600, 300));
    ack_with_size(&mut f, id, &surface, (595, 300));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (888, 300),
        "rounded down: on to the dock, not back to c's edge"
    );
    ack_with_size(&mut f, id, &surface, (883, 300));

    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(last_configured(&mut f, id, &surface), (600, 300));
    ack_with_size(&mut f, id, &surface, (605, 300));

    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (250, 300),
        "rounded up: on to d's edge, not back to c's"
    );
    assert_eq!(window_position(&mut f, &a), Point::from((100, 100)));
}

/// What a grow would cover is judged from the frame the client committed, not
/// the one it was asked for: a window docked against the rounded edge sits
/// between the two, and growing on would cover it.
#[test]
fn resize_to_snap_stops_at_a_window_docked_against_a_rounded_answer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    // Below `a`, offering its right edge at 700 as an alignment.
    window_at(&mut f, id, "c", (100, 412), (600, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (600, 300));
    ack_with_size(&mut f, id, &surface, (585, 300));
    // Docked snap.gap off the edge `a` committed, at 685.
    window_at(&mut f, id, "n", (697, 100), (400, 300));
    focus(&mut f, &a);
    // The refocus configure carries the size asked for again, and the client
    // rounds it the same way.
    ack_with_size(&mut f, id, &surface, (585, 300));
    park_view(&mut f, Point::from((0.0, 0.0)));

    let configures = configure_count(&mut f, id, &surface);
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        configure_count(&mut f, id, &surface),
        configures,
        "docked against n already: no configure"
    );
}

/// A rounded answer that lands exactly on another stop has reached it: the
/// walk goes on past it rather than re-asking for the size the window already
/// has.
#[test]
fn resize_to_snap_counts_a_stop_the_rounding_landed_on_as_reached() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    // Above and below `a`, offering their right edges at 350 and 983.
    window_at(&mut f, id, "d", (100, -212), (250, 300));
    window_at(&mut f, id, "c", (100, 412), (883, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (883, 300));
    ack_with_size(&mut f, id, &surface, (880, 300));
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (888, 300));
    ack_with_size(&mut f, id, &surface, (883, 300));

    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (250, 300),
        "sitting on c's edge already, the walk goes on to d's"
    );
}

/// The fence is the usable area in canvas px, so at zoom 0.5 it reaches twice
/// as far — and `outer_gap` stays canvas px, as `fill-window` measures it.
#[test]
fn resize_to_snap_grows_to_the_fill_edge_at_the_current_zoom() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 12.0\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    f.state().with_output_state(|os| os.zoom = 0.5);

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(last_configured(&mut f, id, &surface), (3728, 300));
    act(&mut f, Action::ResizeToSnap(Direction::Down));
    assert_eq!(last_configured(&mut f, id, &surface), (3728, 2048));
}

/// A title bar sits above the content: the bottom edge the walk moves is the
/// content's own, and a neighbour's frame starts a bar above its content.
#[test]
fn resize_to_snap_down_measures_title_bars() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    let (b, _) = window_at(&mut f, id, "b", (100, 700), (400, 300));
    // Beside `a`, offering its bottom edge at 300 as an alignment.
    window_at(&mut f, id, "c", (512, 100), (400, 200));
    give_ssd(&mut f, &a);
    give_ssd(&mut f, &b);
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::ResizeToSnap(Direction::Down));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (400, 563),
        "b's frame starts at 675, so the bottom docks at 663"
    );
    assert_eq!(window_position(&mut f, &a), Point::from((100, 100)));

    let configures = configure_count(&mut f, id, &surface);
    act(&mut f, Action::ResizeToSnap(Direction::Down));
    assert_eq!(configure_count(&mut f, id, &surface), configures);

    act(&mut f, Action::ResizeToSnap(Direction::Up));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (400, 200),
        "up to c's bottom edge, the bar staying above the content"
    );
}

#[test]
fn a_stand_in_resizes_and_moves_to_snap_like_a_client() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((100, 100)),
        Size::from((400, 300)),
        "s",
        "S",
    );
    f.state().focus_and_raise_suspended(sid);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let s = f.state().find_suspended(sid).unwrap();
    let elem = StageWindow::Suspended(s.clone());

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(s.size.get(), Size::from((888, 300)));
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(s.size.get(), Size::from((888, 300)), "docked: a no-op");

    // Docked at snap.gap already, the next stop right is the neighbour's far
    // side.
    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        f.state().stage.position_of(&elem),
        Some(Point::from((1412, 100))),
        "the stand-in's bar sits above its body, so the horizontal stop is a client's"
    );

    f.state().dismiss_suspended(sid);
}

/// The action prelude exits fullscreen and only *sends* the restore, so
/// committed geometry is still the output's size when the move measures.
#[test]
fn move_to_snap_during_fullscreen_measures_the_restored_window() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let key = super::server_surface(&a).id();

    f.state().enter_fullscreen(&a, Some(output));
    f.double_roundtrip(id);
    super::adopt_last_configure(&mut f, id, &surface);
    assert!(f.state().is_fullscreen(), "precondition: fullscreen");

    act(&mut f, Action::MoveToSnap(Direction::Right));

    assert!(!f.state().is_fullscreen());
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((588, 100)),
        "docked by its restored 400 px width, not the 1920 px it still commits"
    );
    assert!(
        f.state().camera_target().is_none() && f.state().zoom_target().is_none(),
        "and judged in view by that width too, so the camera stays"
    );
    assert!(
        !f.state().pending_recenter.contains_key(&key),
        "the exit's owed recenter went with the move"
    );
}

#[test]
fn resize_to_snap_leaves_a_window_outside_the_viewport_alone() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((3000.0, 0.0)));
    let configures = configure_count(&mut f, id, &surface);

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        configure_count(&mut f, id, &surface),
        configures,
        "off screen: no configure"
    );

    park_view(&mut f, Point::from((0.0, 0.0)));
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (1820, 300),
        "back in view, the same press grows to the usable edge"
    );
}

#[test]
fn a_stand_in_moves_down_by_its_frame_and_lands_its_body_below_the_bar() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    window_at(&mut f, id, "b", (100, 1000), (400, 300));
    let sid = f.state().insert_suspended_for_test(
        1,
        Point::from((100, 100)),
        Size::from((400, 300)),
        "s",
        "S",
    );
    f.state().focus_and_raise_suspended(sid);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let s = f.state().find_suspended(sid).unwrap();
    let elem = StageWindow::Suspended(s);

    act(&mut f, Action::MoveToSnap(Direction::Down));

    // Frame 75..400 (a 25 px bar) docks its bottom at 988: origin 663, body
    // 25 lower.
    assert_eq!(
        f.state().stage.position_of(&elem),
        Some(Point::from((100, 688)))
    );

    f.state().dismiss_suspended(sid);
}

#[test]
fn a_border_widens_the_frames_the_stops_measure() {
    let mut f = Fixture::with_config(config("[decorations]\nborder_width = 2\n"));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));

    act(&mut f, Action::MoveToSnap(Direction::Right));
    assert_eq!(
        window_position(&mut f, &a),
        Point::from((584, 100)),
        "frames 98..502 and 998..1402 dock at 986, so the content starts at 584"
    );

    act(&mut f, Action::ResizeToSnap(Direction::Down));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (400, 980),
        "the frame grows to 1082, a border past the usable edge at outer_gap 0"
    );
}
