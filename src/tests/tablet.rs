//! Graphics-tablet (`wp_tablet_manager_v2`) input. A pen drives the seat
//! pointer — cursor, hover focus, hot corners, click-to-focus and the grabs
//! all run off it, so server-side decorations and legacy apps keep working —
//! but the client under the pen gets exactly one stream: tablet-v2 when it
//! has the tool bound, the emulated `wl_pointer` otherwise. Most of what
//! follows asserts on pointer-side bookkeeping rather than tablet-v2 wire
//! traffic.
//!
//! The per-device output resolver is shared with touch
//! (`DriftWm::touch_output_for_device`); the fake reports no libinput device,
//! so resolution falls through to the config override or the first output.

use smithay::input::pointer::CursorImageStatus;
use smithay::input::tablet::{TabletSeatHandler, TabletSeatTrait};
use smithay::utils::{Logical, Point};
use wayland_protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use driftwm::config::BTN_LEFT;

use crate::state::StageWindow;

use super::client::{ClientId, DataDeviceEvent, LayerConfigureProps, TabletEvent, TabletToolEvent};
use super::input_backend::{
    FakeDevice, FakeTabletAxes, fake_tool, key_press, pen_proximity_in, pen_proximity_in_screen,
    pen_proximity_out, pen_tip_down, pen_tip_up, pen_to, pen_to_screen_with, pen_to_with,
    pointer_to, tablet_added, tablet_removed, touch_down, trackpad_scroll,
};
use super::popups::grow_popup;
use super::{
    Fixture, config, keyboard_focus, map_window, pointer_focus, server_surface, window_by_app_id,
};

/// evdev code of the left Super key, the default `mod_key`.
const KEY_LEFTMETA: u32 = 125;

/// A registered pen in proximity — the state every scenario below starts from.
fn pen_in_proximity(f: &mut Fixture, at: Point<f64, Logical>) -> FakeDevice {
    let device = FakeDevice::tablet();
    tablet_added(f, &device);
    pen_proximity_in(f, &device, at);
    device
}

#[test]
fn pen_motion_moves_the_pointer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    // Pinned so the expected canvas point is hand-computed rather than run back
    // through the inverse of the mapping under test: at zoom 1 a screen point
    // lands on `screen + camera`.
    f.state().set_camera(Point::from((-100.0, -50.0)));
    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);

    let screen = Point::from((500.0, 400.0));
    pen_proximity_in_screen(&mut f, &device, screen);
    pen_to_screen_with(&mut f, &device, screen, FakeTabletAxes::default());

    assert_eq!(
        f.state().seat.get_pointer().unwrap().current_location(),
        Point::from((400.0, 350.0)),
        "a pen in proximity drives the same seat pointer legacy apps read"
    );
}

#[test]
fn pen_motion_still_drives_the_pointer_before_the_tablet_registers() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let device = FakeDevice::tablet();
    let at = Point::from((40.0, -20.0));

    // No `tablet_added`, so the tablet-v2 half of the handler finds no tablet
    // to forward to.
    pen_to(&mut f, &device, at);

    // The pointer half runs first and unconditionally, so a missed registration
    // degrades to pointer-only input rather than to a dead pen.
    assert_eq!(
        f.state().seat.get_pointer().unwrap().current_location(),
        at,
        "the pointer half of a pen motion must not depend on tablet registration"
    );
}

#[test]
fn pen_tip_down_focuses_the_window_under_it() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "pen-target", (400, 300));
    let window = window_by_app_id(&mut f, "pen-target").expect("mapped");
    let pos = f.state().stage.position_of(&window).expect("staged");
    let at = Point::from((f64::from(pos.x) + 200.0, f64::from(pos.y) + 150.0));

    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    pen_tip_down(&mut f, &device);

    assert_eq!(
        keyboard_focus(&mut f).as_ref(),
        Some(&server_surface(&window)),
        "a tip-down is routed through the pointer button path, so it focuses \
         the window under the pen like a click would"
    );
}

#[test]
fn pen_motion_records_what_it_delivered() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "pen-target", (400, 300));
    let window = window_by_app_id(&mut f, "pen-target").expect("mapped");
    let pos = f.state().stage.position_of(&window).expect("staged");
    let at = Point::from((f64::from(pos.x) + 200.0, f64::from(pos.y) + 150.0));

    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    f.roundtrip(id);

    let screen = {
        let camera = f.state().camera();
        let zoom = f.state().zoom();
        driftwm::canvas::canvas_to_screen(driftwm::canvas::CanvasPos(at), camera, zoom).0
    };
    let (focus, origin) = f
        .state()
        .pointer_focus_under_pick(screen, at)
        .expect("the window must be under the pen, or this scenario tests nothing");

    // `refresh_pointer_focus` skips a resync when the delivery it would make
    // matches this record, so a pen that moves the pointer without writing it
    // leaves a later resync comparing against a delivery that never happened.
    assert_eq!(
        f.state().last_pointer_delivery,
        Some((focus, origin, at - origin)),
        "pen motion must record what it delivered, like every other path that \
         moves the pointer"
    );
}

#[test]
fn pen_motion_takes_over_the_output_it_maps_to() {
    let mut f = Fixture::with_config(config(
        r#"
        [input.tablet]
        map_to_output = "HEADLESS-2"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));
    assert_eq!(
        f.state().focused_output.as_ref(),
        Some(&out1),
        "the first output must start active, or this scenario tests nothing"
    );

    let at = Point::from((0.0, 0.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);

    // A device pinned to one output makes that output active, or everything
    // reading `active_output()` afterwards acts on the wrong monitor.
    assert_eq!(
        f.state().focused_output.as_ref(),
        Some(&out2),
        "a pen pinned to an output must make that output active"
    );
}

#[test]
fn pen_motion_checks_hot_corners() {
    let mut f = Fixture::with_config(config(
        r#"
        [[outputs]]
        name = "*"
        [outputs.hot_corners]
        top_left = "zoom-out"
        "#,
    ));
    f.add_output(1, (1920, 1080));

    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);
    let output = f.state().active_output().expect("an output");

    // Raw screen space: a corner is a screen-space zone, and which canvas point
    // lands on it depends on the camera.
    let corner = Point::from((2.0, 2.0));
    pen_proximity_in_screen(&mut f, &device, corner);
    pen_to_screen_with(&mut f, &device, corner, FakeTabletAxes::default());

    assert!(
        crate::state::output_state(&output).zoom_target.is_some(),
        "a pen entering a hot corner must arm it, like pointer motion does"
    );
}

#[test]
fn pen_motion_respects_pick_mode() {
    let mut f = Fixture::with_config(config(
        r#"
        [zoom]
        interact_min = 0.5
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "pen-target", (400, 300));
    let window = window_by_app_id(&mut f, "pen-target").expect("mapped");
    f.state().map_window(
        crate::state::StageWindow::Client(window.clone()),
        Point::from((500, 400)),
        true,
    );
    f.state().set_camera(Point::from((0.0, 0.0)));
    f.state().set_zoom(0.3);

    let at = Point::from((700.0, 550.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    f.roundtrip(id);

    // Below `interact_min` a canvas window takes no pointer input — clicks pick
    // or move it instead — and a pen drives the seat pointer like any mouse.
    assert!(
        super::pointer_focus(&mut f).is_none(),
        "in pick mode a pen must not hand the window pointer focus, or its \
         clicks reach the client while a mouse's would pick the window"
    );

    // Positive control: the same pen over the same window above the threshold
    // must reach it, or the assertion above would also pass on a pen that
    // simply never found the window.
    f.state().set_zoom(0.8);
    pen_to(&mut f, &device, at);
    f.roundtrip(id);
    assert!(
        super::pointer_focus(&mut f).is_some(),
        "above the threshold the pen reaches the window normally"
    );
}

#[test]
fn pen_proximity_takes_over_the_output_it_maps_to() {
    let mut f = Fixture::with_config(config(
        r#"
        [input.tablet]
        map_to_output = "HEADLESS-2"
        "#,
    ));
    let out1 = f.add_output(1, (1920, 1080));
    let out2 = f.add_output(2, (1920, 1080));
    assert_eq!(
        f.state().focused_output.as_ref(),
        Some(&out1),
        "the first output must start active, or this scenario tests nothing"
    );

    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);
    pen_proximity_in(&mut f, &device, Point::from((0.0, 0.0)));

    // Proximity resolves focus through the same cascade as motion, and that
    // cascade reads `active_output()` — so entering proximity has to claim the
    // output too, or the pen hit-tests its coordinates against another
    // monitor's layers and pins before it has moved once.
    assert_eq!(
        f.state().focused_output.as_ref(),
        Some(&out2),
        "entering proximity claims the pen's output, like motion does"
    );
}

#[test]
fn a_tip_without_a_registered_tool_still_clicks() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let device = FakeDevice::tablet();

    // No `tablet_added`, so the seat has no tool to send `tip_down` to.
    pen_tip_down(&mut f, &device);

    assert!(
        f.state().held_buttons.contains(&BTN_LEFT),
        "an absent tool must not swallow the emulated button, or a pen whose \
         registration was missed moves the cursor but can never click"
    );
}

#[test]
fn pen_motion_restores_a_cursor_touch_hid() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let at = Point::from((40.0, -20.0));

    touch_down(&mut f, at, 0);
    assert!(
        f.state().cursor.hidden_by_touch,
        "touch hides the cursor, or this scenario tests nothing"
    );

    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);

    assert!(
        !f.state().cursor.hidden_by_touch,
        "a pen is real pointer input, so it brings the cursor back"
    );
}

#[test]
fn a_tip_cycle_presses_and_releases_the_left_button() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let at = Point::from((40.0, -20.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);

    pen_tip_down(&mut f, &device);
    assert!(
        f.state().held_buttons.contains(&BTN_LEFT),
        "a tip-down is emulated as a left press for clients that speak no tablet protocol"
    );

    pen_tip_up(&mut f, &device);
    assert!(
        !f.state().held_buttons.contains(&BTN_LEFT),
        "a tip-up must release it again, or the next click lands on a stuck button"
    );
}

#[test]
fn removing_a_tablet_takes_it_off_the_seat() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let device = FakeDevice::tablet();

    tablet_added(&mut f, &device);
    assert_eq!(
        f.state().seat.tablet_seat().count_tablets(),
        1,
        "a device advertising TabletTool registers on the seat"
    );

    tablet_removed(&mut f, &device);
    assert_eq!(
        f.state().seat.tablet_seat().count_tablets(),
        0,
        "unplugging it takes it back off"
    );
}

#[test]
fn a_tablet_plugged_in_while_locked_still_registers() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.client(id).lock_session();
    f.roundtrip(id);
    assert!(
        f.state().session_lock.is_locked(),
        "the lock handler ran, or this scenario tests nothing"
    );

    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);

    // Registration is bookkeeping, not input delivery. Dropping it with the
    // rest of the locked event stream would leave the tablet dead after unlock
    // until it was physically replugged.
    assert_eq!(
        f.state().seat.tablet_seat().count_tablets(),
        1,
        "a hotplug behind the lock screen still reaches the seat"
    );
}

/// A stroke as a client that speaks no tablet protocol sees it: the tip goes
/// down, the pen moves, the tip lifts. Every motion in between has to reach
/// the client's `wl_pointer`, or a drawing app draws a dot where a line was.
#[test]
fn a_pen_stroke_delivers_every_motion_to_a_pointer_only_client() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "canvas", (800, 600));
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let origin = f.state().stage.position_of(&window).unwrap().to_f64();

    let start = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, start);
    pen_to(&mut f, &device, start);
    f.double_roundtrip(id);
    f.client(id).state.pointer_positions.clear();
    f.client(id).state.pointer_buttons.clear();

    pen_tip_down(&mut f, &device);
    let stroke = stroke_from(start);
    for at in &stroke {
        pen_to(&mut f, &device, *at);
    }
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);

    let buttons = &f.client(id).state.pointer_buttons;
    assert_eq!(
        buttons.len(),
        2,
        "the tip cycle must reach the client as one press and one release: {buttons:?}"
    );
    let positions = &f.client(id).state.pointer_positions;
    assert_eq!(
        positions.len(),
        stroke.len(),
        "every pen motion between tip-down and tip-up must reach the client as \
         a pointer motion; fewer means the stroke collapses to dots: {positions:?}"
    );
    let expected: Vec<(f64, f64)> = stroke
        .iter()
        .map(|at| ((at.x - origin.x), (at.y - origin.y)))
        .collect();
    assert_eq!(
        *positions, expected,
        "the motions must arrive in order at the pen's surface-local positions"
    );
}

/// A mapped window on a client that also speaks tablet-v2, with the window's
/// canvas origin for turning pen positions into the surface-local ones the
/// client should see. Round-trips so the compositor has processed the tablet
/// seat request before any pen event runs.
///
/// The window maps under the fixture's parked pointer and is seated on
/// `wl_pointer` as by a mouse; that delivery is cleared so what remains is
/// the pen's alone.
fn tablet_client_with_window(
    f: &mut Fixture,
) -> (
    ClientId,
    wayland_client::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
) {
    let id = f.add_client();
    let surface = map_window(f, id, "canvas", (800, 600));
    f.client(id).get_tablet_seat();
    f.double_roundtrip(id);
    f.client(id).state.pointer_positions.clear();
    let window = window_by_app_id(f, "canvas").unwrap();
    let origin = f.state().stage.position_of(&window).unwrap().to_f64();
    (id, surface, origin)
}

/// A repeat `DeviceAdded` for a known tablet (a session resume, a libinput
/// re-enumeration) must not read to clients as an unplug and a replug.
#[test]
fn a_repeated_device_add_does_not_replace_a_known_tablet() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.client(id).get_tablet_seat();
    f.roundtrip(id);

    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);
    f.roundtrip(id);
    assert_eq!(
        f.client(id).state.tablet_events,
        vec![TabletEvent::Added],
        "precondition: the first add announces the tablet once"
    );

    tablet_added(&mut f, &device);
    f.roundtrip(id);
    assert_eq!(
        f.client(id).state.tablet_events,
        vec![TabletEvent::Added],
        "a repeat add of the same tablet must announce nothing"
    );
}

/// Eight pen positions along a diagonal from `start`.
fn stroke_from(start: Point<f64, Logical>) -> Vec<Point<f64, Logical>> {
    (1..=8)
        .map(|i| start + Point::from((i as f64 * 20.0, i as f64 * 10.0)))
        .collect()
}

/// The motion a tablet client should see for a pen at canvas `at` over a
/// window whose origin is `origin`.
fn motion_at(at: Point<f64, Logical>, origin: Point<f64, Logical>) -> TabletToolEvent {
    TabletToolEvent::Motion {
        x: at.x - origin.x,
        y: at.y - origin.y,
    }
}

/// Everything the tool sent after its `done`: type and capabilities have to
/// finish before a client may act on the tool, so its stream starts there.
fn after_done(events: Vec<TabletToolEvent>) -> Vec<TabletToolEvent> {
    let done = events
        .iter()
        .position(|e| *e == TabletToolEvent::Done)
        .unwrap_or_else(|| {
            panic!("the tool must finish its announce with `done` before anything else: {events:?}")
        });
    events[done + 1..].to_vec()
}

/// The stream cut into frames: the protocol makes everything before a `frame`
/// one hardware event, so a stroke reads as one group per pen event. Panics
/// on a trailing group no frame closed — a client never acts on one.
fn frames(events: &[TabletToolEvent]) -> Vec<Vec<TabletToolEvent>> {
    let mut groups: Vec<Vec<TabletToolEvent>> = events
        .split(|e| *e == TabletToolEvent::Frame)
        .map(<[TabletToolEvent]>::to_vec)
        .collect();
    let open = groups.pop().unwrap_or_default();
    assert!(
        open.is_empty(),
        "events after the last frame never reach the app: {open:?}"
    );
    groups
}

/// [`frames`] with the final frame — the tip lifting — returned separately.
/// The grab that held the stroke to one surface re-checks focus as it lifts,
/// which can put a motion at the pen's resting position beside the `up`, so
/// the lift is checked for its `up` and the stroke frames exactly.
fn stroke_and_lift(
    events: &[TabletToolEvent],
) -> (Vec<Vec<TabletToolEvent>>, Vec<TabletToolEvent>) {
    let mut frames = frames(events);
    let lift = frames.pop().unwrap_or_default();
    (frames, lift)
}

/// The same stroke as a tablet-aware client sees it over `wp_tablet_v2`,
/// which is what a browser-engine drawing app consumes. Every pen event has to
/// arrive as its own frame at the pen's surface-local position: a motion that
/// never comes, or two sharing a frame, is a stroke reduced to dots.
#[test]
fn a_pen_stroke_reaches_a_tablet_client_as_a_continuous_sequence() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);

    let start = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, start);
    pen_to(&mut f, &device, start);
    pen_tip_down(&mut f, &device);
    let stroke = stroke_from(start);
    for at in &stroke {
        pen_to(&mut f, &device, *at);
    }
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);

    let events = after_done(f.client(id).drain_tablet_tool_events());
    let mut expected = vec![
        vec![
            TabletToolEvent::ProximityIn { surface },
            motion_at(start, origin),
        ],
        vec![motion_at(start, origin)],
        vec![TabletToolEvent::Down],
    ];
    expected.extend(stroke.iter().map(|at| vec![motion_at(*at, origin)]));
    let (frames, lift) = stroke_and_lift(&events);
    assert!(
        lift.contains(&TabletToolEvent::Up),
        "the stroke must close with the tip lifting in its final frame: {lift:?}"
    );
    assert_eq!(
        frames, expected,
        "a stroke must reach a tablet client as proximity, the hover, tip down, \
         then one motion frame per pen event; a frame short is a motion the app \
         never draws"
    );
}

/// A client with the tool bound takes the pen on tablet-v2 alone. Fed the
/// emulated pointer as well, a drawing app reads the tip as a second press at
/// the same spot and restarts the stroke, so every stroke is a dot.
#[test]
fn a_tablet_client_gets_the_stroke_on_tablet_v2_and_nothing_on_wl_pointer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);

    let start = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, start);
    pen_to(&mut f, &device, start);
    pen_tip_down(&mut f, &device);
    let stroke = stroke_from(start);
    for at in &stroke {
        pen_to(&mut f, &device, *at);
    }
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);

    // The stroke is what lies between the tip going down and lifting.
    let events = f.client(id).drain_tablet_tool_events();
    let tablet_motions: Vec<(f64, f64)> = events
        .iter()
        .skip_while(|e| **e != TabletToolEvent::Down)
        .take_while(|e| **e != TabletToolEvent::Up)
        .filter_map(|e| match e {
            TabletToolEvent::Motion { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect();
    let expected: Vec<(f64, f64)> = stroke
        .iter()
        .map(|at| (at.x - origin.x, at.y - origin.y))
        .collect();
    assert_eq!(
        tablet_motions, expected,
        "the tablet stream must carry the whole stroke, or the silence asserted \
         below is a dead pen rather than a withheld one: {events:?}"
    );

    let buttons = &f.client(id).state.pointer_buttons;
    assert!(
        buttons.is_empty(),
        "a client on the tablet stream must see no emulated button: {buttons:?}"
    );
    let positions = &f.client(id).state.pointer_positions;
    assert!(
        positions.is_empty(),
        "nor any wl_pointer enter or motion: {positions:?}"
    );
}

/// The pull re-picks focus at the cursor every iteration and every frame. It
/// has to withhold the pen's client the same way the motion did, or the
/// client is re-seated on `wl_pointer` one pump after the pen left it out.
#[test]
fn the_pull_does_not_re_seat_wl_pointer_on_a_tablet_client_under_the_pen() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);

    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    assert!(
        pointer_focus(&mut f).is_none(),
        "precondition: the motion itself withheld the client"
    );

    f.pump(3);
    f.double_roundtrip(id);

    assert!(
        pointer_focus(&mut f).is_none(),
        "the pull must not hand the client the focus the motion withheld"
    );
    let positions = &f.client(id).state.pointer_positions;
    assert!(
        positions.is_empty(),
        "no wl_pointer enter may reach the client across the pulls: {positions:?}"
    );
}

/// A pen and a mouse share the seat pointer; whichever moved it last owns it.
/// A mouse motion after a pen hover puts the client back on `wl_pointer`, and
/// the next pen motion takes it off again.
#[test]
fn mouse_motion_after_a_pen_hover_hands_wl_pointer_back() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);
    let window = window_by_app_id(&mut f, "canvas").unwrap();

    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    f.double_roundtrip(id);
    assert!(
        f.client(id).state.pointer_positions.is_empty(),
        "precondition: the pen hover reached the client on tablet-v2 only"
    );

    let mouse = FakeDevice::mouse();
    pointer_to(&mut f, &mouse, at + Point::from((10.0, 0.0)));
    f.double_roundtrip(id);
    assert_eq!(
        pointer_focus(&mut f),
        Some(server_surface(&window)),
        "a mouse reclaims the pointer, so the client is on wl_pointer again"
    );
    assert_eq!(
        f.client(id).state.pointer_positions,
        vec![(110.0, 100.0)],
        "the reclaiming motion enters the client at the mouse's position"
    );

    pen_to(&mut f, &device, at);
    f.double_roundtrip(id);
    assert!(
        pointer_focus(&mut f).is_none(),
        "the pen's next motion takes the pointer back and withholds the client again"
    );
}

/// Withholding `wl_pointer` changes what the client receives, not what the
/// compositor does with the tip: it still runs through the button path, so a
/// tap on an unfocused window raises and focuses it like a click would.
#[test]
fn a_pen_tip_on_a_tablet_client_still_raises_and_focuses_it() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (_id, _surface, origin) = tablet_client_with_window(&mut f);
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let other = f.add_client();
    map_window(&mut f, other, "other", (300, 300));
    let other_window = window_by_app_id(&mut f, "other").unwrap();
    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&other_window)),
        "precondition: the window mapped last holds focus"
    );
    assert_eq!(
        f.state().stage.windows().next_back(),
        Some(&StageWindow::Client(other_window.clone())),
        "precondition: and sits on top"
    );

    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    pen_tip_down(&mut f, &device);
    pen_tip_up(&mut f, &device);

    assert_eq!(
        keyboard_focus(&mut f),
        Some(server_surface(&window)),
        "the tip focuses the window under the pen"
    );
    assert_eq!(
        f.state().stage.windows().next_back(),
        Some(&StageWindow::Client(window)),
        "and raises it"
    );
}

/// A grabbed popup on the tablet client's window, covering the pen's spot:
/// tapped once for a serial, then the popup opened, grabbed and grown past
/// its single-pixel input region. The default positioner centres it on the
/// parent's origin.
fn tablet_client_popup_under_pen(
    f: &mut Fixture,
    id: ClientId,
    surface: &wayland_client::protocol::wl_surface::WlSurface,
    device: &FakeDevice,
) -> wayland_client::protocol::wl_surface::WlSurface {
    pen_tip_down(f, device);
    pen_tip_up(f, device);
    f.double_roundtrip(id);

    let popup = f.client(id).create_popup(surface);
    let popup_surface = popup.surface.clone();
    popup.commit();
    f.roundtrip(id);
    let popup = f.client(id).popup(&popup_surface);
    popup.grab(1);
    popup.attach_new_buffer();
    popup.ack_last_and_commit();
    f.double_roundtrip(id);
    grow_popup(f, id, &popup_surface, (200, 100));
    assert!(
        f.state().popup_grab.is_some(),
        "precondition: the popup holds a grab, or this scenario tests nothing"
    );
    popup_surface
}

/// A pen tap on a withheld client's own menu carries no `wl_pointer` focus on
/// the popup's client, which smithay's popup grab would read as a click
/// outside. It has to leave the menu standing, or no menu works with a pen.
#[test]
fn a_pen_tap_on_a_tablet_clients_own_popup_leaves_it_standing() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);

    let at = origin + Point::from((10.0, 10.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    let popup_surface = tablet_client_popup_under_pen(&mut f, id, &surface, &device);
    f.client(id).drain_tablet_tool_events();

    pen_to(&mut f, &device, origin + Point::from((12.0, 12.0)));
    pen_tip_down(&mut f, &device);
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);
    f.pump(3);

    assert!(
        !f.client(id).popup(&popup_surface).popup_done,
        "a tap on the menu must not dismiss it"
    );
    assert!(
        f.state().popup_grab.is_some(),
        "and the grab that routes its input stays"
    );
    let events = f.client(id).drain_tablet_tool_events();
    assert!(
        events.contains(&TabletToolEvent::Down),
        "the tap reaches the client on tablet-v2, which is why the emulated \
         button has nothing left to do: {events:?}"
    );
}

/// The counterpart: a pen tap beside the menu is a click outside, and the
/// menu goes.
#[test]
fn a_pen_tap_outside_a_tablet_clients_popup_dismisses_it() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);

    let at = origin + Point::from((10.0, 10.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    let popup_surface = tablet_client_popup_under_pen(&mut f, id, &surface, &device);

    // Right of the 800-wide window: bare canvas.
    pen_to(&mut f, &device, origin + Point::from((900.0, 100.0)));
    pen_tip_down(&mut f, &device);
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);
    f.pump(3);

    assert!(
        f.client(id).popup(&popup_surface).popup_done,
        "a tap outside the menu dismisses it, as a click does"
    );
}

/// Hover the pen over the tablet client and have that client hide the tool
/// cursor, as a drawing canvas does — what its `set_cursor` on the tool
/// arrives as.
fn pen_over_tablet_client_hiding_the_cursor(
    f: &mut Fixture,
    id: ClientId,
    origin: Point<f64, Logical>,
) -> FakeDevice {
    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(f, at);
    pen_to(f, &device, at);
    f.double_roundtrip(id);
    f.state()
        .tablet_tool_image(&fake_tool(), CursorImageStatus::Hidden);
    assert_eq!(
        f.state().cursor.cursor_status,
        CursorImageStatus::Hidden,
        "precondition: the client's tool cursor is in effect"
    );
    device
}

/// smithay drops a client's cursor on the `wl_pointer` leave, which a client
/// withheld from `wl_pointer` never gets. The image its tool set has to go
/// when the tool leaves it, or the cursor stays hidden over the bare canvas.
#[test]
fn the_tool_cursor_is_dropped_when_the_pen_moves_off_a_tablet_client() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);
    let device = pen_over_tablet_client_hiding_the_cursor(&mut f, id, origin);

    // Right of the 800-wide window: bare canvas.
    pen_to(&mut f, &device, origin + Point::from((900.0, 100.0)));
    f.double_roundtrip(id);

    assert_eq!(
        f.state().cursor.cursor_status,
        CursorImageStatus::default_named(),
        "the tool leaving the client restores the default cursor"
    );
}

#[test]
fn the_tool_cursor_is_dropped_when_the_pen_leaves_proximity() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);
    let device = pen_over_tablet_client_hiding_the_cursor(&mut f, id, origin);

    pen_proximity_out(&mut f, &device);

    assert_eq!(
        f.state().cursor.cursor_status,
        CursorImageStatus::default_named(),
        "lifting the pen away restores the default cursor"
    );
}

/// A press over a layer grants an on-demand layer keyboard focus, resolved
/// from the pointer's focus. A pen withholds that focus from a bar whose
/// client has the tool bound, so the tap has to resolve the bar from the pick
/// instead, or a pen can never type into a bar's search box.
#[test]
fn a_pen_tap_still_grants_an_on_demand_layer_keyboard_focus() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.client(id).get_tablet_seat();
    let created = f
        .client(id)
        .create_layer(None, zwlr_layer_shell_v1::Layer::Top, "bar");
    let layer_surface = created.surface.clone();
    created.set_configure_props(LayerConfigureProps {
        size: Some((200, 150)),
        kb_interactivity: Some(zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand),
        ..Default::default()
    });
    created.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&layer_surface);
    layer.set_size(200, 150);
    layer.attach_new_buffer();
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
    assert!(
        keyboard_focus(&mut f).is_none(),
        "precondition: an on-demand layer takes no focus until pressed"
    );

    // Unanchored, so the layer centres on the output.
    let device = FakeDevice::tablet();
    tablet_added(&mut f, &device);
    let on_bar = Point::from((960.0, 540.0));
    pen_proximity_in_screen(&mut f, &device, on_bar);
    pen_to_screen_with(&mut f, &device, on_bar, FakeTabletAxes::default());
    f.double_roundtrip(id);
    assert!(
        pointer_focus(&mut f).is_none(),
        "precondition: the bar's client has the tool bound, so the pen withheld it"
    );

    pen_tip_down(&mut f, &device);
    pen_tip_up(&mut f, &device);

    let output = f.state().space.outputs().next().cloned().unwrap();
    let layer_root = smithay::desktop::layer_map_for_output(&output)
        .layers()
        .next()
        .unwrap()
        .wl_surface()
        .clone();
    assert_eq!(
        keyboard_focus(&mut f),
        Some(layer_root),
        "the tap grants the bar keyboard focus, resolved from the pick"
    );
}

/// A drawing app reads pressure per motion, so a motion whose pressure change
/// went missing draws at the previous width — or at none. Each pen event with
/// a pressure change has to reach the client as one frame carrying both.
#[test]
fn a_pen_stroke_with_pressure_reports_pressure_per_motion() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);

    let start = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, start);
    pen_to(&mut f, &device, start);
    f.double_roundtrip(id);
    f.client(id).drain_tablet_tool_events();

    pen_tip_down(&mut f, &device);
    let stroke = stroke_from(start);
    // Eighths are exact in binary, so the wire value below is not a rounding
    // question.
    let pressures: Vec<f64> = (1..=8).map(|i| f64::from(i) / 8.0).collect();
    for (at, pressure) in stroke.iter().zip(&pressures) {
        pen_to_with(
            &mut f,
            &device,
            *at,
            FakeTabletAxes {
                pressure: Some(*pressure),
                ..FakeTabletAxes::default()
            },
        );
    }
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);

    let events = f.client(id).drain_tablet_tool_events();
    let mut expected = vec![vec![TabletToolEvent::Down]];
    expected.extend(stroke.iter().zip(&pressures).map(|(at, pressure)| {
        vec![
            // The protocol normalises pressure to 0..=65535.
            TabletToolEvent::Pressure((pressure * 65535.0) as u32),
            motion_at(*at, origin),
        ]
    }));
    let (frames, lift) = stroke_and_lift(&events);
    assert!(
        lift.contains(&TabletToolEvent::Up),
        "the stroke must close with the tip lifting in its final frame: {lift:?}"
    );
    assert_eq!(
        frames, expected,
        "every motion frame of a pressured stroke must carry that motion's \
         pressure"
    );
}

/// A client that asks for the tablet seat while the pen is already over its
/// window must be told so, or it waits for a proximity_in that only comes
/// after the pen leaves and returns — every event until then lands on a tool
/// it believes is out of proximity.
#[test]
fn a_tablet_client_binding_late_still_gets_proximity() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "canvas", (800, 600));
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let origin = f.state().stage.position_of(&window).unwrap().to_f64();

    let at = origin + Point::from((100.0, 100.0));
    pen_in_proximity(&mut f, at);
    f.double_roundtrip(id);
    assert!(
        f.client(id).drain_tablet_tool_events().is_empty(),
        "no tool exists before the seat is requested, or this scenario tests nothing"
    );

    f.client(id).get_tablet_seat();
    f.double_roundtrip(id);

    let events = after_done(f.client(id).drain_tablet_tool_events());
    let frames = frames(&events);
    assert!(
        frames
            .first()
            .is_some_and(|frame| frame.contains(&TabletToolEvent::ProximityIn { surface })),
        "a tool already over the window when the client binds must open with \
         proximity_in: {frames:?}"
    );
}

/// A second tablet-aware client with a window of its own, for scenarios where
/// the pen and the client acting on the seat differ. It maps centred inside
/// the first client's window, which a tip on that window raises over it, so
/// it is moved out to the right of it. Fails loudly if the window landed
/// under `pen_at`, which would make the pen its client too.
fn another_tablet_client_with_window(
    f: &mut Fixture,
    app_id: &str,
    pen_at: Point<f64, Logical>,
) -> (
    ClientId,
    wayland_client::protocol::wl_surface::WlSurface,
    Point<f64, Logical>,
) {
    let id = f.add_client();
    let surface = map_window(f, id, app_id, (300, 300));
    f.client(id).get_tablet_seat();
    f.double_roundtrip(id);
    let window = window_by_app_id(f, app_id).unwrap();
    f.state()
        .stage
        .set_position(&window, Point::from((450, -150)));
    let origin = f.state().stage.position_of(&window).unwrap().to_f64();
    let covers = pen_at.x >= origin.x
        && pen_at.x < origin.x + 300.0
        && pen_at.y >= origin.y
        && pen_at.y < origin.y + 300.0;
    assert!(
        !covers,
        "the second window must sit clear of the pen, or this scenario tests nothing"
    );
    (id, surface, origin)
}

/// A pen with its tip down on the tablet client's window, and the serial that
/// tip's `down` carried — the one a client cites to start an interactive
/// request from it.
fn pen_tip_down_on_tablet_client(
    f: &mut Fixture,
    id: ClientId,
    at: Point<f64, Logical>,
) -> (FakeDevice, u32) {
    let device = pen_in_proximity(f, at);
    pen_to(f, &device, at);
    pen_tip_down(f, &device);
    f.double_roundtrip(id);
    let serial =
        f.client(id).state.tablet_down_serial.expect(
            "the tip must reach the client as a tablet down, or there is no serial to cite",
        );
    (device, serial)
}

/// A tablet client is withheld from `wl_pointer`, so the seat pointer's grab
/// has no focus to match its move request against. The pen's own `down` serial
/// is the only proof of a press the client has, and it has to be enough, or a
/// CSD title bar cannot be dragged with a pen.
#[test]
fn a_tablet_client_can_start_a_move_from_its_pen_tip() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let at = origin + Point::from((100.0, 100.0));
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, id, at);

    f.client(id).window(&surface).move_(serial);
    f.roundtrip(id);
    let before = f.state().stage.position_of(&window).unwrap();
    pen_to(&mut f, &device, at + Point::from((60.0, 40.0)));

    assert_eq!(
        f.state().stage.position_of(&window).unwrap(),
        before + Point::from((60, 40)),
        "the window must follow the pen while its tip is down"
    );

    // The move belongs to the stroke: once the tip lifts the pen is just
    // hovering, and a window that kept following it would be unplaceable.
    pen_tip_up(&mut f, &device);
    let released = f.state().stage.position_of(&window).unwrap();
    pen_to(&mut f, &device, at + Point::from((200.0, 120.0)));

    assert_eq!(
        f.state().stage.position_of(&window).unwrap(),
        released,
        "a hovering pen must not keep dragging the window"
    );
}

/// Same gap as the move: the resize request is matched against a pointer grab
/// whose start focus a withheld client never has.
#[test]
fn a_tablet_client_can_start_a_resize_from_its_pen_tip() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);
    let at = origin + Point::from((790.0, 590.0));
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, id, at);

    f.client(id)
        .window(&surface)
        .resize(ResizeEdge::BottomRight, serial);
    f.roundtrip(id);
    pen_to(&mut f, &device, at + Point::from((100.0, 80.0)));
    f.double_roundtrip(id);

    let size = f
        .client(id)
        .window(&surface)
        .configures_received
        .last()
        .unwrap()
        .1
        .size;
    assert!(
        size.0 > 800 && size.1 > 600,
        "the pen dragging the corner must configure the window larger than 800x600, got {size:?}"
    );
}

/// A serial is only proof of a press for the client that was sent it. Another
/// tablet client citing the pen's `down` serial must not take the window grab
/// for itself, or any client could move its own window off someone else's
/// stroke. Holds with and without the pen-serial acceptance.
#[test]
fn another_client_cannot_start_a_move_from_the_pens_serial() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);
    let at = origin + Point::from((100.0, 100.0));
    let (other, other_surface, _) = another_tablet_client_with_window(&mut f, "other", at);
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, id, at);
    let other_window = window_by_app_id(&mut f, "other").unwrap();
    let before = f.state().stage.position_of(&other_window).unwrap();

    f.client(other).window(&other_surface).move_(serial);
    f.roundtrip(other);
    pen_to(&mut f, &device, at + Point::from((60.0, 40.0)));

    assert_eq!(
        f.state().stage.position_of(&other_window).unwrap(),
        before,
        "a window not under the pen must not follow it"
    );
}

/// smithay accepts a drag only when the seat pointer's grab carries the cited
/// serial, and a tablet client has only its tip's to cite. Once started, the
/// drag finds its target in what lies under the pen, which a pen withholding
/// tablet clients from `wl_pointer` must not hide from it, or nothing can be
/// dropped onto one.
#[test]
fn a_pen_can_drag_and_drop_between_tablet_clients() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (a, a_surface, a_origin) = tablet_client_with_window(&mut f);
    let at = a_origin + Point::from((100.0, 100.0));
    let (b, b_surface, b_origin) = another_tablet_client_with_window(&mut f, "other", at);
    f.client(a).get_data_device();
    f.client(b).get_data_device();
    f.double_roundtrip(a);
    f.double_roundtrip(b);
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, a, at);
    f.client(b).state.pointer_positions.clear();

    f.client(a).start_drag(&a_surface, serial);
    f.roundtrip(a);
    pen_to(&mut f, &device, b_origin + Point::from((100.0, 100.0)));
    f.double_roundtrip(b);

    assert!(
        f.client(b)
            .state
            .data_device_events
            .contains(&DataDeviceEvent::Enter { surface: b_surface }),
        "dragging the pen onto another window must offer the drop to it: {:?}",
        f.client(b).state.data_device_events
    );
    let positions = &f.client(b).state.pointer_positions;
    assert!(
        positions.is_empty(),
        "the drag reaches it on wl_data_device alone, not on wl_pointer: {positions:?}"
    );
}

/// A scroll-pan slides windows under a cursor that has not moved, and the pen
/// still owns the pointer then. Seating the client on `wl_pointer` at that
/// point hands a tablet client the second stream it is withheld from, and every
/// scroll event flaps it in and out.
#[test]
fn a_scroll_pan_does_not_seat_a_tablet_client_under_the_pen() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);

    let above = origin + Point::from((100.0, -30.0));
    let device = pen_in_proximity(&mut f, above);
    pen_to(&mut f, &device, above);
    key_press(&mut f, KEY_LEFTMETA);
    let touchpad = FakeDevice::touchpad();
    for _ in 0..20 {
        trackpad_scroll(&mut f, &touchpad);
    }
    let cursor = f.state().seat.get_pointer().unwrap().current_location();
    assert!(
        cursor.y > origin.y,
        "precondition: the pan must have slid the window under the cursor, now at {cursor:?}"
    );

    f.pump(3);
    f.double_roundtrip(id);

    let positions = &f.client(id).state.pointer_positions;
    assert!(
        positions.is_empty(),
        "no wl_pointer enter or motion may reach a client the pen withholds: {positions:?}"
    );
}

/// A client with no tablet seat gets the pen as a pointer, so its cursor image
/// is a plain `wl_pointer` one. The pen leaving proximity is not the pointer
/// leaving the client, and resetting the cursor to the arrow there overrides
/// what a pointer-only app asked for.
#[test]
fn proximity_out_keeps_a_pointer_only_clients_cursor() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "canvas", (800, 600));
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let origin = f.state().stage.position_of(&window).unwrap().to_f64();
    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    f.double_roundtrip(id);

    f.client(id).hide_pointer_cursor();
    f.double_roundtrip(id);
    assert_eq!(
        f.state().cursor.cursor_status,
        CursorImageStatus::Hidden,
        "precondition: the client's own cursor is in effect"
    );

    pen_proximity_out(&mut f, &device);

    assert_eq!(
        f.state().cursor.cursor_status,
        CursorImageStatus::Hidden,
        "the pointer-only client's cursor must survive the pen leaving proximity"
    );
}

/// A mouse motion puts the client back on `wl_pointer`, but the pen's next tip
/// still belongs to tablet-v2 alone. The tip carries no motion to re-assert
/// pen ownership, so the tip itself must, or the emulated left button reaches
/// the client beside the tablet `down` and a drawing app restarts its stroke.
#[test]
fn a_tip_after_a_mouse_motion_stays_off_wl_pointer() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, _surface, origin) = tablet_client_with_window(&mut f);
    let at = origin + Point::from((100.0, 100.0));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    pointer_to(&mut f, &FakeDevice::mouse(), at + Point::from((10.0, 0.0)));
    f.double_roundtrip(id);
    assert!(
        pointer_focus(&mut f).is_some(),
        "precondition: the mouse put the client back on wl_pointer"
    );
    f.client(id).state.pointer_buttons.clear();
    f.client(id).drain_tablet_tool_events();

    pen_tip_down(&mut f, &device);
    pen_tip_up(&mut f, &device);
    f.double_roundtrip(id);

    let tablet = f.client(id).drain_tablet_tool_events();
    assert!(
        tablet.contains(&TabletToolEvent::Down),
        "the tip must still reach the client on tablet-v2: {tablet:?}"
    );
    let buttons = &f.client(id).state.pointer_buttons;
    assert!(
        buttons.is_empty(),
        "the tip must not also arrive as an emulated wl_pointer button: {buttons:?}"
    );
}

/// A press a compositor binding consumes still reaches a tablet client as the
/// tool's `down`. That serial must not let the client swap the binding's grab
/// for a move of its own: Super and a pen drag on a window pan the canvas, as
/// they do with a mouse, whatever the client asks for under the tip.
#[test]
fn a_tablet_client_cannot_turn_a_bound_pen_drag_into_a_move() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);
    let window = window_by_app_id(&mut f, "canvas").unwrap();
    let at = origin + Point::from((100.0, 100.0));
    key_press(&mut f, KEY_LEFTMETA);
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, id, at);
    let before = f.state().stage.position_of(&window).unwrap();
    let camera = f.state().camera();

    f.client(id).window(&surface).move_(serial);
    f.roundtrip(id);
    pen_to(&mut f, &device, at + Point::from((60.0, 40.0)));

    assert_eq!(
        f.state().stage.position_of(&window).unwrap(),
        before,
        "the client must not take the binding's drag for a window move"
    );
    assert_ne!(
        f.state().camera(),
        camera,
        "the pan the binding started carries on"
    );
}

/// The drag counterpart: smithay starts a drag for any pointer grab holding
/// the cited serial, so a binding's grab must not hold the tip's.
#[test]
fn a_tablet_client_cannot_turn_a_bound_pen_drag_into_a_drag_and_drop() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);
    f.client(id).get_data_device();
    f.double_roundtrip(id);
    let at = origin + Point::from((100.0, 100.0));
    key_press(&mut f, KEY_LEFTMETA);
    let (device, serial) = pen_tip_down_on_tablet_client(&mut f, id, at);
    let camera = f.state().camera();

    f.client(id).start_drag(&surface, serial);
    f.roundtrip(id);
    pen_to(&mut f, &device, at + Point::from((60.0, 40.0)));

    assert_ne!(
        f.state().camera(),
        camera,
        "the pan the binding started carries on instead of a drag"
    );
}

/// A tip with no pen motion since the mouse moved presses where the mouse
/// left the pointer, on whatever client is there, while the tool's `down`
/// goes to the client under the pen. That press must not carry the tip's
/// serial, or the pen's client could cite it to turn the other client's click
/// into a drag of its own.
#[test]
fn a_tip_pressing_on_another_client_does_not_lend_it_the_tips_serial() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let (id, surface, origin) = tablet_client_with_window(&mut f);
    f.client(id).get_data_device();
    f.double_roundtrip(id);
    let at = origin + Point::from((100.0, 100.0));
    let plain = f.add_client();
    map_window(&mut f, plain, "plain", (300, 300));
    let plain_window = window_by_app_id(&mut f, "plain").unwrap();
    f.state()
        .stage
        .set_position(&plain_window, Point::from((450, -150)));
    let device = pen_in_proximity(&mut f, at);
    pen_to(&mut f, &device, at);
    pointer_to(&mut f, &FakeDevice::mouse(), Point::from((600.0, 0.0)));
    f.client(plain).state.pointer_buttons.clear();

    pen_tip_down(&mut f, &device);
    f.double_roundtrip(id);
    f.double_roundtrip(plain);
    assert_eq!(
        f.client(plain).state.pointer_buttons,
        vec![(BTN_LEFT, 1)],
        "precondition: the press lands on the client under the mouse"
    );
    let serial = f
        .client(id)
        .state
        .tablet_down_serial
        .expect("precondition: the tool's down reaches the pen's client");

    f.client(id).start_drag(&surface, serial);
    f.roundtrip(id);
    pen_to(&mut f, &device, at + Point::from((10.0, 0.0)));
    f.double_roundtrip(id);

    assert!(
        f.client(id).state.data_device_events.is_empty(),
        "no drag may start off another client's press: {:?}",
        f.client(id).state.data_device_events
    );
}
