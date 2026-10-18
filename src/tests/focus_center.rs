//! `focus-center` centers and focuses what the pointer is on. Over empty
//! canvas there is nothing to aim at, so it centers the focused window as
//! `center-window` does; over a pin it focuses the pin and leaves the camera,
//! which has no canvas position to travel to for it.

use driftwm::config::Action;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::state::StageWindow;

use super::{Fixture, config, map_window, settle, to_canvas, window_by_app_id};

const PIN_RULE: &str = r#"
[[window_rules]]
app_id = "pin"
pinned_to_screen = true
size = [400, 300]
"#;

fn pt(x: f64, y: f64) -> Point<f64, Logical> {
    Point::from((x, y))
}

/// Park the viewport at the canvas origin, zoom 1, nothing in flight.
fn origin_view(f: &mut Fixture) {
    f.state().with_output_state(|os| {
        os.camera = Point::from((0.0, 0.0));
        os.camera_target = None;
        os.zoom = 1.0;
        os.zoom_target = None;
        os.zoom_animation_anchor = None;
    });
}

#[test]
fn over_empty_canvas_it_centers_the_focused_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();
    map_window(&mut f, id, "w", (300, 200));
    let window = window_by_app_id(&mut f, "w").unwrap();
    let element = StageWindow::Client(window.clone());
    f.state()
        .map_window(element.clone(), Point::from((4000, 3000)), false);
    origin_view(&mut f);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&window, serial);
    assert!(
        !f.state().window_fully_in_viewport(&window),
        "precondition: the focused window is off screen"
    );

    f.state().warp_pointer(pt(960.0, 540.0));
    f.state().execute_action(&Action::FocusCenter);
    settle(&mut f);

    let rect = f.state().visual_frame_rect(&element).unwrap();
    let (camera, zoom) = (f.state().camera(), f.state().zoom());
    let center = pt(
        ((rect.x_low + rect.x_high) / 2.0 - camera.x) * zoom,
        ((rect.y_low + rect.y_high) / 2.0 - camera.y) * zoom,
    );
    let vc = f.state().usable_center_screen();
    assert!(
        (center.x - vc.x).abs() < 1e-6 && (center.y - vc.y).abs() < 1e-6,
        "the camera landed on the focused window: {center:?} vs {vc:?}"
    );
    assert_eq!(f.state().focused_window(), Some(window));
}

#[test]
fn over_a_pin_it_focuses_the_pin_and_leaves_the_camera() {
    let mut f = Fixture::with_config(config(PIN_RULE));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "pin", (400, 300));
    let pin = window_by_app_id(&mut f, "pin").unwrap();
    let site = f.state().stage.pin_of(&pin).unwrap().screen_pos;
    let on_pin = to_canvas(&mut f, pt(site.x as f64 + 200.0, site.y as f64 + 150.0));

    // A canvas window drawn behind the pin, under the pointer too.
    map_window(&mut f, id, "beneath", (400, 300));
    let beneath = window_by_app_id(&mut f, "beneath").unwrap();
    let origin = Point::from((on_pin.x as i32 - 80, on_pin.y as i32 - 70));
    f.state()
        .map_window(StageWindow::Client(beneath.clone()), origin, false);
    let serial = SERIAL_COUNTER.next_serial();
    f.state().raise_and_focus(&beneath, serial);
    settle(&mut f);
    let camera = f.state().camera();

    f.state().warp_pointer(on_pin);
    f.state().execute_action(&Action::FocusCenter);

    assert_eq!(f.state().focused_window(), Some(pin), "the pin took focus");
    assert_eq!(f.state().camera_target(), None, "no pan was armed");
    assert_eq!(f.state().camera(), camera);
}
