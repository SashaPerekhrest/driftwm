//! Keyboard snapping: `move-to-snap` walks the focused element to the stops
//! drag snapping offers, `resize-to-snap` walks its right or bottom edge
//! between them and the usable area's edge — clients and stand-ins alike.

use smithay::desktop::Window;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Size};
use wayland_client::protocol::wl_surface::WlSurface as ClientSurface;

use super::client::ClientId;
use super::{
    Fixture, config, configure_count, last_configured, map_window, settle, window_by_app_id,
    window_position,
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
        Point::from((500, 100)),
        "right edges aligned: nothing further"
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
fn resize_to_snap_walks_the_right_edge_out_and_back() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let (a, surface) = window_at(&mut f, id, "a", (100, 100), (400, 300));
    window_at(&mut f, id, "b", (1000, 100), (400, 300));
    focus(&mut f, &a);
    park_view(&mut f, Point::from((0.0, 0.0)));
    let before = window_position(&mut f, &a);

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (888, 300),
        "the right edge stops snap.gap short of the neighbour"
    );
    assert_eq!(window_position(&mut f, &a), before, "the left edge holds");

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (1820, 300),
        "then the usable area's right edge"
    );

    let configures = configure_count(&mut f, id, &surface);
    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        configure_count(&mut f, id, &surface),
        configures,
        "nothing past the screen: no configure at all"
    );

    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (888, 300),
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
    assert_eq!(s.size.get(), Size::from((1820, 300)));
    act(&mut f, Action::ResizeToSnap(Direction::Left));
    assert_eq!(s.size.get(), Size::from((888, 300)));

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

    act(&mut f, Action::ResizeToSnap(Direction::Right));
    assert_eq!(
        last_configured(&mut f, id, &surface),
        (1336, 300),
        "the frame grows to 1922, a border past the usable edge at outer_gap 0"
    );
}
