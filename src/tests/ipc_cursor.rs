//! `driftwm msg cursor` reports the pointer in the global output layout: screen
//! space, Y-down, offset by its output's layout position, whatever camera and
//! zoom that output views the canvas through.

use smithay::utils::Point;

use crate::ipc::dispatch;
use crate::ipc::protocol::{Request, Response};

use super::Fixture;
use super::input_backend::{FakeDevice, pointer_to_screen};

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
