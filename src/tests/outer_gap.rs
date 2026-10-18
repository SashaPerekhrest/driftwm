//! `[snap] outer_gap`: fill measures the inset from the usable area to a
//! window's title bar / content edge, not to its visual frame — the border
//! lies outside that inset, off screen at `outer_gap = 0`. Fit ignores it: a
//! fitted window's content covers the usable area edge to edge, its border
//! hanging off screen.

use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::StageWindow;

use super::{
    Fixture, TICK, adopt_last_configure, config, map_top_panel, map_window, settle,
    tick_until_settled, window_by_app_id,
};

/// Fit `window`, deliver the configure, ack it at the size the compositor
/// asked for, and settle the resulting camera/zoom animation — the shape
/// `configure_sequences.rs::fit_and_settle` drives, needed here because
/// `window.geometry().size` only reflects the fit's target size once the
/// client has acked it.
fn fit_and_adopt(
    f: &mut Fixture,
    window: &Window,
    id: super::client::ClientId,
    surface: &wayland_client::protocol::wl_surface::WlSurface,
) {
    f.state().fit_window(window);
    f.double_roundtrip(id);
    adopt_last_configure(f, id, surface);
    f.double_roundtrip(id);
    f.state().tick_window_animations(TICK);
    settle(f);
}

/// `window`'s content rect (no chrome) in screen space, at the active
/// output's current camera and zoom.
fn content_screen_rect(f: &mut Fixture, window: &Window) -> Rectangle<f64, Logical> {
    let loc = f.state().stage.position_of(window).unwrap();
    let size = window.geometry().size;
    let camera = f.state().camera();
    let zoom = f.state().zoom();
    Rectangle::new(
        Point::from((
            (loc.x as f64 - camera.x) * zoom,
            (loc.y as f64 - camera.y) * zoom,
        )),
        Size::from((size.w as f64 * zoom, size.h as f64 * zoom)),
    )
}

/// `window`'s visual frame (border + SSD title bar + content) in screen
/// space, as `focus_placement.rs::screen_edges`.
fn frame_screen_rect(f: &mut Fixture, window: &Window) -> Rectangle<f64, Logical> {
    let elem = StageWindow::Client(window.clone());
    let rect = f.state().visual_frame_rect(&elem).unwrap();
    let camera = f.state().camera();
    let zoom = f.state().zoom();
    Rectangle::new(
        Point::from((
            (rect.x_low - camera.x) * zoom,
            (rect.y_low - camera.y) * zoom,
        )),
        Size::from((
            (rect.x_high - rect.x_low) * zoom,
            (rect.y_high - rect.y_low) * zoom,
        )),
    )
}

fn assert_rect_eq(actual: Rectangle<f64, Logical>, expected: Rectangle<i32, Logical>, msg: &str) {
    let expected = expected.to_f64();
    assert!(
        (actual.loc.x - expected.loc.x).abs() < 1e-6
            && (actual.loc.y - expected.loc.y).abs() < 1e-6
            && (actual.size.w - expected.size.w).abs() < 1e-6
            && (actual.size.h - expected.size.h).abs() < 1e-6,
        "{msg}: got {actual:?}, expected {expected:?}"
    );
}

/// A fit lands the content edge exactly on the usable area and hangs the
/// border off the far side of it, rather than inside, whatever `outer_gap`.
#[test]
fn fit_with_a_border_lands_content_on_the_usable_area_whatever_the_outer_gap() {
    let mut f = Fixture::with_config(config(
        r#"
        [snap]
        outer_gap = 20.0

        [decorations]
        border_width = 4
        "#,
    ));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();

    let surface = map_window(&mut f, id, "a", (800, 600));
    let window = window_by_app_id(&mut f, "a").unwrap();

    fit_and_adopt(&mut f, &window, id, &surface);

    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    assert_rect_eq(content, usable, "content must span the usable area exactly");

    let frame = frame_screen_rect(&mut f, &window);
    let expected_frame = Rectangle::new(
        Point::from((usable.loc.x - 4, usable.loc.y - 4)),
        Size::from((usable.size.w + 8, usable.size.h + 8)),
    );
    assert_rect_eq(
        frame,
        expected_frame,
        "the border hangs 4px past the usable area on every side",
    );
}

/// With no border, a fit collapses frame and content onto the usable area
/// alike, whatever `outer_gap`.
#[test]
fn fit_with_no_border_lands_frame_and_content_flush_whatever_the_outer_gap() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 20.0\n"));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();

    let surface = map_window(&mut f, id, "a", (800, 600));
    let window = window_by_app_id(&mut f, "a").unwrap();

    fit_and_adopt(&mut f, &window, id, &surface);

    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    assert_rect_eq(content, usable, "content must span the usable area exactly");

    let frame = frame_screen_rect(&mut f, &window);
    assert_rect_eq(
        frame,
        usable,
        "with no border the frame is the content rect",
    );
}

/// A cluster fit grows the window to the same edge-to-edge rect, pushing its
/// neighbour along, whatever `outer_gap`.
#[test]
fn a_cluster_fit_covers_the_usable_area_whatever_the_outer_gap() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 20.0\n"));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();

    let surface = map_window(&mut f, id, "a", (400, 300));
    let window = window_by_app_id(&mut f, "a").unwrap();
    map_window(&mut f, id, "b", (300, 300));
    let neighbour = window_by_app_id(&mut f, "b").unwrap();
    let gap = f.state().config.snap_gap as i32;
    f.state()
        .map_window(window.clone(), Point::from((900, 300)), false);
    f.state().map_window(
        neighbour.clone(),
        Point::from((900 + 400 + gap, 300)),
        false,
    );
    settle(&mut f);
    tick_until_settled(&mut f);
    let docked = f.state().stage.position_of(&neighbour).unwrap();

    f.state().fit_window_snapped(&window);
    f.double_roundtrip(id);
    adopt_last_configure(&mut f, id, &surface);
    f.double_roundtrip(id);
    f.state().tick_window_animations(TICK);
    settle(&mut f);

    assert_ne!(
        f.state().stage.position_of(&neighbour).unwrap(),
        docked,
        "precondition: the fit pushed the neighbour along"
    );
    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    assert_rect_eq(content, usable, "content must span the usable area exactly");
}

/// A panel's exclusive zone shrinks the usable area fit reads from, not the
/// bare output rect: at `outer_gap = 0` the content edge tracks that shrunk
/// area, landing at the panel's own bottom edge.
#[test]
fn fit_respects_a_panels_exclusive_zone_not_the_bare_output_rect() {
    let mut f = Fixture::with_config(config("[snap]\nouter_gap = 0.0\n"));
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();

    map_top_panel(&mut f, id, 100);
    assert_eq!(
        f.state().get_usable_area().size,
        Size::from((1920, 980)),
        "precondition: the panel claimed a 100px exclusive zone"
    );

    let surface = map_window(&mut f, id, "a", (800, 600));
    let window = window_by_app_id(&mut f, "a").unwrap();

    fit_and_adopt(&mut f, &window, id, &surface);

    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    assert_rect_eq(
        content,
        usable,
        "content must span the panel-shrunk usable area, top edge at y=100",
    );
}

/// At `outer_gap = 0` a lone window's fill lands its content on the usable
/// area — the same rect a fit grows to.
#[test]
fn fill_on_a_lone_window_lands_content_on_the_usable_area() {
    let mut f = Fixture::with_config(config(
        r#"
        [snap]
        outer_gap = 0.0

        [decorations]
        border_width = 4
        "#,
    ));
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    // Fill silently no-ops once the window has fallen off the active
    // viewport, so map and fill immediately, before any camera move.
    let surface = map_window(&mut f, id, "a", (800, 600));
    let window = window_by_app_id(&mut f, "a").unwrap();

    f.state().fill_window(&window);
    f.double_roundtrip(id);
    adopt_last_configure(&mut f, id, &surface);
    f.double_roundtrip(id);

    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    assert_rect_eq(
        content,
        usable,
        "a lone window's fill must land on the content rect fit would produce",
    );
}

/// The default `outer_gap` is 12, and a lone window's fill stops that far in
/// from the usable area on every side — the maximize with a margin.
#[test]
fn fill_on_a_lone_window_stops_the_default_12_px_in() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let id = f.add_client();

    // As above: fill before anything moves the camera off the window.
    let surface = map_window(&mut f, id, "a", (800, 600));
    let window = window_by_app_id(&mut f, "a").unwrap();

    f.state().fill_window(&window);
    f.double_roundtrip(id);
    adopt_last_configure(&mut f, id, &surface);
    f.double_roundtrip(id);

    let usable = f.state().get_usable_area();
    let content = content_screen_rect(&mut f, &window);
    let expected = Rectangle::new(
        Point::from((usable.loc.x + 12, usable.loc.y + 12)),
        Size::from((usable.size.w - 24, usable.size.h - 24)),
    );
    assert_rect_eq(
        content,
        expected,
        "a lone window's fill must stop 12px in from the usable area",
    );
    assert_eq!(f.state().config.snap_outer_gap, 12.0);
}

/// An odd content height (no SSD bar to balance it) can leave the pre-round
/// camera on a half-pixel value; it must still land on a whole pixel, with
/// the content edge exactly on `usable.loc`.
#[test]
fn fit_rounds_the_camera_to_a_whole_pixel_even_with_an_odd_height_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    f.skip_baseline_check();
    let id = f.add_client();

    let surface = map_window(&mut f, id, "a", (800, 601));
    let window = window_by_app_id(&mut f, "a").unwrap();

    fit_and_adopt(&mut f, &window, id, &surface);

    let camera = f.state().camera();
    assert!(
        camera.x.fract() == 0.0 && camera.y.fract() == 0.0,
        "the fit camera must land on a whole pixel, got {camera:?}"
    );

    let usable = f.state().get_usable_area().to_f64();
    let content = content_screen_rect(&mut f, &window);
    assert!(
        (content.loc.x - usable.loc.x).abs() < 1e-6 && (content.loc.y - usable.loc.y).abs() < 1e-6,
        "content edge must land exactly on usable.loc, got {content:?}"
    );
}
