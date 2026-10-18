//! `driftwm msg cursor` reports the pointer in the global output layout: screen
//! space, Y-down, offset by its output's layout position, whatever camera and
//! zoom that output views the canvas through.

use driftwm::canvas::{CanvasPos, canvas_to_screen};
use smithay::output::Output;
use smithay::utils::Point;

use crate::ipc::dispatch;
use crate::ipc::protocol::{Request, Response};

use super::input_backend::{FakeDevice, pointer_relative_motion, pointer_to, pointer_to_screen};
use super::{Fixture, config, map_window, window_by_app_id};

fn read_cursor(f: &mut Fixture) -> (f64, f64, String) {
    match dispatch(Request::Cursor, f.state()) {
        Ok(Response::Cursor { x, y, output }) => (x, y, output),
        other => panic!("expected a Cursor reply, got {other:?}"),
    }
}

fn approx(got: (f64, f64), want: (f64, f64), tol: f64) -> bool {
    (got.0 - want.0).abs() < tol && (got.1 - want.1).abs() < tol
}

/// One output that the config lays out at (2560, 0), viewing the canvas at
/// zoom 0.5 from a camera off the origin.
fn output_laid_out_off_the_origin() -> (Fixture, Output) {
    let mut f = Fixture::with_config(config(
        "[[outputs]]\nname = \"HEADLESS-1\"\nposition = [2560, 0]\n",
    ));
    let output = f.add_output(1, (1920, 1080));
    assert_eq!(
        output.current_location(),
        Point::from((2560, 0)),
        "precondition: the output advertises its configured position"
    );
    f.state().with_output_state(|os| {
        os.zoom = 0.5;
        os.camera = Point::from((-700.0, 300.0));
    });
    (f, output)
}

/// `cursor` reports the pointer in the global output layout, not canvas space:
/// the camera and zoom must not leak into it, the output's layout offset must.
#[test]
fn cursor_reports_screen_space_in_the_output_layout() {
    let (mut f, output) = output_laid_out_off_the_origin();
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

/// A locked pointer is held where it is drawn, so the reply stays on that spot
/// while the mouse keeps moving.
#[test]
fn cursor_holds_still_under_a_pointer_lock() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let surface = map_window(&mut f, id, "game", (800, 600));
    let window = window_by_app_id(&mut f, "game").unwrap();
    let center =
        f.state().stage.position_of(&window).unwrap().to_f64() + Point::from((400.0, 300.0));
    pointer_to(&mut f, &FakeDevice::mouse(), center);
    let _lock = f.client(id).lock_pointer(&surface);
    f.double_roundtrip(id);
    assert!(
        f.state().pointer_constraint_locked(),
        "precondition: the lock is active"
    );
    let held = canvas_to_screen(CanvasPos(center), f.state().camera(), f.state().zoom()).0;

    pointer_relative_motion(&mut f, &FakeDevice::mouse(), Point::from((40.0, 25.0)));

    let (x, y, _) = read_cursor(&mut f);
    assert!(
        approx((x, y), (held.x, held.y), 0.01),
        "the cursor is held at {held:?} on an output laid out at the origin, got ({x}, {y})"
    );
}

/// Locking turns the stored pointer location into screen space, so the camera
/// and zoom must not be applied to it a second time: the reply reads the same
/// across the lock, and follows the pointer around the lock screen.
#[test]
fn cursor_reads_the_same_across_a_session_lock() {
    let (mut f, output) = output_laid_out_off_the_origin();
    let id = f.add_client();
    pointer_to_screen(&mut f, &FakeDevice::mouse(), Point::from((300.0, 200.0)));
    let before = read_cursor(&mut f);

    f.client(id).lock_session();
    f.roundtrip(id);
    assert!(f.state().session_lock.is_locked(), "precondition: locked");
    assert_eq!(read_cursor(&mut f), before, "the lock moved nothing");

    pointer_to_screen(&mut f, &FakeDevice::mouse(), Point::from((500.0, 600.0)));
    let (x, y, name) = read_cursor(&mut f);
    assert!(
        approx((x, y), (3060.0, 600.0), 0.01),
        "a locked pointer at screen (500, 600) on an output laid out at \
         (2560, 0), got ({x}, {y})"
    );
    assert_eq!(name, output.name());
}
