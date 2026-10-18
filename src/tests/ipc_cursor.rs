//! `driftwm msg cursor` reports the pointer in the global output layout: screen
//! space, Y-down, offset by its output's layout position, whatever camera and
//! zoom that output views the canvas through.

use smithay::utils::Point;

use crate::ipc::dispatch;
use crate::ipc::protocol::{Request, Response};

use super::input_backend::{FakeDevice, pointer_relative_motion, pointer_to, pointer_to_screen};
use super::{Fixture, map_window, window_by_app_id};

fn read_cursor(f: &mut Fixture) -> (f64, f64, String) {
    match dispatch(Request::Cursor, f.state()) {
        Ok(Response::Cursor { x, y, output }) => (x, y, output),
        other => panic!("expected a Cursor reply, got {other:?}"),
    }
}

fn approx(got: (f64, f64), want: (f64, f64), tol: f64) -> bool {
    (got.0 - want.0).abs() < tol && (got.1 - want.1).abs() < tol
}

/// `cursor` reports the pointer in the global output layout, not canvas space:
/// the camera and zoom must not leak into it, the output's layout offset must.
#[test]
fn cursor_reports_screen_space_in_the_output_layout() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));
    f.state().with_output_state(|os| {
        os.zoom = 0.5;
        os.camera = Point::from((-700.0, 300.0));
        os.layout_position = Point::from((2560, 0));
    });
    pointer_to_screen(&mut f, &FakeDevice::mouse(), Point::from((300.0, 200.0)));

    let (x, y, name) = read_cursor(&mut f);
    assert!(approx((x, y), (2860.0, 200.0), 0.01), "got ({x}, {y})");
    assert_eq!(name, output.name());
}

/// Crossing onto a second output reports that output, offset by the position
/// it advertises to clients: past the first output's *logical* width at scale
/// 2, with the second output's own camera and zoom kept out of the number.
#[test]
fn cursor_follows_the_pointer_onto_a_second_output() {
    let mut f = Fixture::new();
    f.add_output_scaled(1, (2560, 1440), 2.0);
    let second = f.add_output(2, (1920, 1080));
    assert_eq!(
        second.current_location(),
        Point::from((1280, 0)),
        "precondition: the second output sits past the first's logical width"
    );
    {
        let mut os = crate::state::output_state(&second);
        os.zoom = 0.5;
        os.camera = Point::from((-700.0, 300.0));
    }

    pointer_to_screen(&mut f, &FakeDevice::mouse(), Point::from((1200.0, 100.0)));
    pointer_relative_motion(&mut f, &FakeDevice::mouse(), Point::from((180.0, 50.0)));

    let (x, y, name) = read_cursor(&mut f);
    assert_eq!(name, second.name());
    assert!(
        approx((x, y), (1380.0, 150.0), 0.01),
        "(1200, 100) on the first output moved by (180, 50), got ({x}, {y})"
    );
}

/// A locked pointer is held where it is drawn, so the reply holds still too
/// while the mouse keeps moving.
#[test]
fn cursor_holds_still_under_a_pointer_lock() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "game", (800, 600));
    let window = window_by_app_id(&mut f, "game").unwrap();
    let position = f.state().stage.position_of(&window).unwrap().to_f64();
    pointer_to(
        &mut f,
        &FakeDevice::mouse(),
        position + Point::from((400.0, 300.0)),
    );
    let _lock = f.client(id).lock_pointer(&surface);
    f.double_roundtrip(id);
    assert!(
        f.state().pointer_constraint_locked(),
        "precondition: the lock is active"
    );

    let before = read_cursor(&mut f);
    pointer_relative_motion(&mut f, &FakeDevice::mouse(), Point::from((40.0, 25.0)));
    assert_eq!(read_cursor(&mut f), before);
}

/// While the session is locked the stored pointer location is already screen
/// space, so the camera and zoom must not be applied to it a second time.
#[test]
fn cursor_while_locked_reports_the_screen_position_in_the_output_layout() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));
    let id = f.add_client();
    f.state().with_output_state(|os| {
        os.zoom = 0.5;
        os.camera = Point::from((-700.0, 300.0));
        os.layout_position = Point::from((2560, 0));
    });
    f.client(id).lock_session();
    f.roundtrip(id);
    assert!(f.state().session_lock.is_locked(), "precondition: locked");
    pointer_to_screen(&mut f, &FakeDevice::mouse(), Point::from((300.0, 200.0)));

    let (x, y, name) = read_cursor(&mut f);
    assert!(
        approx((x, y), (2860.0, 200.0), 0.01),
        "a locked pointer sits at screen (300, 200) on an output laid out at \
         (2560, 0), got ({x}, {y})"
    );
    assert_eq!(name, output.name());
}
