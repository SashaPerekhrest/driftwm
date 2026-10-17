//! Mouse bindings over a screen-space layer surface (a desktop widget on
//! Bottom, a bar on Top). Only `[mouse.anywhere]` bindings apply there: a
//! layer is neither a window nor empty canvas, so on-window and on-canvas
//! bindings stay off, and whatever `anywhere` leaves unbound is the layer's
//! own click or scroll.

use driftwm::config::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use smithay::utils::{Logical, Point, Size};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use crate::grabs::{NavigateGrab, PanGrab};
use crate::state::StageWindow;

use super::client::{ClientId, LayerConfigureProps};
use super::input_backend::{
    FakeDevice, key_press, key_release, pointer_to_screen, press, release, trackpad_scroll,
    wheel_notch_down,
};
use super::{Fixture, TICK, config, map_window, to_canvas, window_by_app_id};

/// evdev codes, the space `key_press` reports in. Super is the default
/// `mod_key`.
const KEY_LEFTALT: u32 = 56;
const KEY_LEFTCTRL: u32 = 29;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTMETA: u32 = 125;

/// `wl_pointer.button_state` on the wire, as the client records it.
const PRESSED: u32 = 1;
const RELEASED: u32 = 0;

/// A widget below every canvas window, and a bar above them.
const LAYERS: [Layer; 2] = [Layer::Bottom, Layer::Top];

/// Where the pointer rests on each layer's surface, in screen coordinates.
/// The bar spans the output's bottom 40 px; the widget is centred.
const ON_BAR: (f64, f64) = (1200.0, 1060.0);
const ON_WIDGET: (f64, f64) = (960.0, 540.0);

const CLICK_ACTION: &str = r#"
[mouse.anywhere]
"mod+right" = "zoom-out"
"#;

/// Bound to the notch alone: no continuous `mod+shift` scroll binding exists,
/// so only the discrete site can zoom.
const NOTCH_ACTION: &str = r#"
[mouse.anywhere]
"mod+shift+wheel-down" = "zoom-out"
"#;

const PICK_MODE: &str = r#"
[zoom]
interact_min = 0.5
"#;

fn cursor_on(layer: Layer) -> Point<f64, Logical> {
    match layer {
        Layer::Top => ON_BAR.into(),
        _ => ON_WIDGET.into(),
    }
}

fn map_layer(f: &mut Fixture, id: ClientId, layer: Layer) {
    let (size, anchor) = match layer {
        Layer::Top => (
            (1920, 40),
            Some(
                zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            ),
        ),
        _ => ((400, 300), None),
    };
    let created = f.client(id).create_layer(None, layer, "shell");
    let surface = created.surface.clone();
    created.set_configure_props(LayerConfigureProps {
        size: Some(size),
        anchor,
        exclusive_zone: Some(0),
        ..Default::default()
    });
    created.commit();
    f.roundtrip(id);

    let mapped = f.client(id).layer(&surface);
    mapped.set_size(size.0 as u16, size.1 as u16);
    mapped.attach_new_buffer();
    mapped.ack_last_and_commit();
    f.double_roundtrip(id);
}

/// Rest the pointer on `layer` and confirm the layer owns it. Without that
/// check a scene that misses the layer would test the canvas instead.
fn point_at_layer(f: &mut Fixture, layer: Layer) {
    pointer_to_screen(f, &FakeDevice::mouse(), cursor_on(layer));
    assert!(
        f.state().pointer_over_layer,
        "{layer:?}: the pointer must rest on the layer"
    );
}

fn pointer_on_layer(f: &mut Fixture, layer: Layer) -> ClientId {
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_layer(f, id, layer);
    point_at_layer(f, layer);
    id
}

fn layer_buttons(f: &mut Fixture, id: ClientId) -> Vec<(u32, u32)> {
    f.double_roundtrip(id);
    f.client(id).state.pointer_buttons.clone()
}

fn layer_axes(f: &mut Fixture, id: ClientId) -> Vec<f64> {
    f.double_roundtrip(id);
    f.client(id).state.pointer_axes.clone()
}

/// Map a 400x300 window and put its centre under the screen point `screen`,
/// clearing the navigate-to flight the map arms so the view stays put.
fn window_under(f: &mut Fixture, screen: Point<f64, Logical>) -> smithay::desktop::Window {
    let id = f.add_client();
    map_window(f, id, "app", (400, 300));
    let window = window_by_app_id(f, "app").unwrap();
    f.state().with_output_state(|os| {
        os.camera_target = None;
        os.zoom_target = None;
    });
    let centre = to_canvas(f, screen);
    let origin = Point::from((centre.x as i32 - 200, centre.y as i32 - 150));
    f.state()
        .map_window(StageWindow::Client(window.clone()), origin, false);
    window
}

/// The default `mod+left` pan works over a layer like anywhere else, and the
/// grab it starts owns the release: the layer hears neither half of the click.
#[test]
fn mod_drag_over_a_layer_pans_the_canvas_and_the_layer_never_hears_it() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);
        let camera = f.state().camera();

        key_press(&mut f, KEY_LEFTMETA);
        press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
        pointer_to_screen(
            &mut f,
            &FakeDevice::mouse(),
            cursor_on(layer) - Point::from((200.0, 0.0)),
        );
        release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

        assert!(
            f.state().camera().x > camera.x,
            "{layer:?}: dragging left with mod held must pan the canvas, camera {camera:?} -> {:?}",
            f.state().camera()
        );
        assert_eq!(
            layer_buttons(&mut f, id),
            Vec::new(),
            "{layer:?}: the pan consumed the press, so the layer must see no button"
        );
    }
}

/// A mod+drag over a layer catches a flight in progress, as a press on the
/// canvas does, even though the pan grab ignores the warps the flight sends it.
#[test]
fn mod_drag_over_a_layer_catches_a_camera_flight() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        pointer_on_layer(&mut f, layer);
        f.state().with_output_state(|os| {
            os.camera_target = Some(Point::from((2000.0, 0.0)));
            os.zoom_target = Some(2.0);
        });

        key_press(&mut f, KEY_LEFTMETA);
        press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
        assert!(
            f.state()
                .seat
                .get_pointer()
                .unwrap()
                .with_grab(|_, g| g.is::<PanGrab>())
                .unwrap_or(false),
            "{layer:?}: precondition: mod+left over a layer starts a PanGrab"
        );

        for _ in 0..10 {
            f.state().apply_zoom_animation(TICK);
            f.state().apply_camera_animation(TICK);
        }
        assert!(
            f.state().camera_target().is_none(),
            "{layer:?}: the pan is called off"
        );
        assert!(
            f.state().zoom_target().is_none(),
            "{layer:?}: and so is the zoom"
        );

        release(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    }
}

/// The default `mod+wheel-scroll` zoom works over a layer, and the notch that
/// drove it never reaches the layer.
#[test]
fn mod_wheel_over_a_layer_zooms_and_the_layer_never_hears_it() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);
        let resting = f.state().zoom_target();

        key_press(&mut f, KEY_LEFTMETA);
        wheel_notch_down(&mut f, &FakeDevice::mouse());

        assert_ne!(
            f.state().zoom_target(),
            resting,
            "{layer:?}: mod+wheel must zoom over a layer"
        );
        assert_eq!(
            layer_axes(&mut f, id),
            Vec::<f64>::new(),
            "{layer:?}: the zoom consumed the scroll, so the layer must see no axis"
        );
    }
}

/// The default `mod+trackpad-scroll` pan works over a layer too.
#[test]
fn mod_trackpad_scroll_over_a_layer_pans_the_canvas() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);
        let camera = f.state().camera();

        key_press(&mut f, KEY_LEFTMETA);
        trackpad_scroll(&mut f, &FakeDevice::touchpad());

        assert_ne!(
            f.state().camera(),
            camera,
            "{layer:?}: mod+trackpad-scroll must pan over a layer"
        );
        assert_eq!(
            layer_axes(&mut f, id),
            Vec::<f64>::new(),
            "{layer:?}: the pan consumed the scroll, so the layer must see no axis"
        );
    }
}

/// An `anywhere` action bound to a button fires over a layer. No grab is left
/// to own the release, so the release must be swallowed as well, or the layer
/// gets a release for a press it never saw.
#[test]
fn an_anywhere_click_action_over_a_layer_swallows_press_and_release() {
    for layer in LAYERS {
        let mut f = Fixture::with_config(config(CLICK_ACTION));
        let id = pointer_on_layer(&mut f, layer);
        let resting = f.state().zoom_target();

        key_press(&mut f, KEY_LEFTMETA);
        press(&mut f, &FakeDevice::mouse(), BTN_RIGHT);
        release(&mut f, &FakeDevice::mouse(), BTN_RIGHT);

        assert_ne!(
            f.state().zoom_target(),
            resting,
            "{layer:?}: the anywhere mod+right action must fire over a layer"
        );
        assert_eq!(
            layer_buttons(&mut f, id),
            Vec::new(),
            "{layer:?}: the action consumed the press, so neither it nor its \
             release may reach the layer"
        );
    }
}

/// An `anywhere` wheel-notch action fires over a layer: the discrete site has
/// its own lookup ahead of the continuous one.
#[test]
fn an_anywhere_wheel_notch_action_fires_over_a_layer() {
    for layer in LAYERS {
        let mut f = Fixture::with_config(config(NOTCH_ACTION));
        let id = pointer_on_layer(&mut f, layer);
        let resting = f.state().zoom_target();

        key_press(&mut f, KEY_LEFTMETA);
        key_press(&mut f, KEY_LEFTSHIFT);
        wheel_notch_down(&mut f, &FakeDevice::mouse());

        assert_ne!(
            f.state().zoom_target(),
            resting,
            "{layer:?}: the anywhere notch action must fire over a layer"
        );
        assert_eq!(
            layer_axes(&mut f, id),
            Vec::<f64>::new(),
            "{layer:?}: the action consumed the notch, so the layer must see no axis"
        );
    }
}

/// A bare click is the layer's. Over empty canvas the same click is the
/// on-canvas pan, which must not leak onto a layer.
#[test]
fn a_bare_click_over_a_layer_reaches_the_layer() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);
        let camera = f.state().camera();

        press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
        pointer_to_screen(
            &mut f,
            &FakeDevice::mouse(),
            cursor_on(layer) - Point::from((200.0, 0.0)),
        );
        release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

        assert_eq!(
            layer_buttons(&mut f, id),
            vec![(BTN_LEFT, PRESSED), (BTN_LEFT, RELEASED)],
            "{layer:?}: an unbound click must reach the layer whole"
        );
        assert_eq!(
            f.state().camera(),
            camera,
            "{layer:?}: the on-canvas pan must not fire over a layer"
        );
    }
}

/// A bare scroll is the layer's. Over empty canvas the same wheel is the
/// on-canvas zoom, which must not leak onto a layer.
#[test]
fn a_bare_scroll_over_a_layer_reaches_the_layer() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);
        let resting = f.state().zoom_target();

        wheel_notch_down(&mut f, &FakeDevice::mouse());

        assert_eq!(
            layer_axes(&mut f, id),
            vec![15.0],
            "{layer:?}: an unbound scroll must reach the layer"
        );
        assert_eq!(
            f.state().zoom_target(),
            resting,
            "{layer:?}: the on-canvas zoom must not fire over a layer"
        );
    }
}

/// On-window bindings stay off over a bar even with a window right beneath it:
/// neither the default `alt+left` move nor the default `mod+middle`
/// toggle-fullscreen may act on the hidden window.
#[test]
fn an_on_window_binding_does_not_fire_over_a_bar_covering_a_window() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let bar = f.add_client();
    map_layer(&mut f, bar, Layer::Top);
    let window = window_under(&mut f, cursor_on(Layer::Top));
    point_at_layer(&mut f, Layer::Top);
    assert_eq!(
        f.state().focused_window(),
        Some(window.clone()),
        "precondition: toggle-fullscreen acts on the focused window"
    );
    let before = f.state().stage.position_of(&window).expect("staged");

    key_press(&mut f, KEY_LEFTALT);
    press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    pointer_to_screen(
        &mut f,
        &FakeDevice::mouse(),
        cursor_on(Layer::Top) - Point::from((200.0, 0.0)),
    );
    release(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    key_release(&mut f, KEY_LEFTALT);

    key_press(&mut f, KEY_LEFTMETA);
    press(&mut f, &FakeDevice::mouse(), BTN_MIDDLE);
    release(&mut f, &FakeDevice::mouse(), BTN_MIDDLE);

    assert_eq!(
        f.state().stage.position_of(&window).expect("staged"),
        before,
        "alt+left is an on-window binding, so it must not move the window under the bar"
    );
    assert!(
        !f.state().is_fullscreen(),
        "mod+middle is an on-window binding, so it must not fullscreen the window under the bar"
    );
    assert_eq!(
        layer_buttons(&mut f, bar),
        vec![
            (BTN_LEFT, PRESSED),
            (BTN_LEFT, RELEASED),
            (BTN_MIDDLE, PRESSED),
            (BTN_MIDDLE, RELEASED)
        ],
        "both unbound clicks are the bar's"
    );
}

/// The default `mod+ctrl+left` center-nearest starts its grab over a layer
/// too, and the grab owns the release.
#[test]
fn mod_ctrl_drag_over_a_layer_starts_center_nearest() {
    for layer in LAYERS {
        let mut f = Fixture::new();
        let id = pointer_on_layer(&mut f, layer);

        key_press(&mut f, KEY_LEFTMETA);
        key_press(&mut f, KEY_LEFTCTRL);
        press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
        let navigating = f
            .state()
            .seat
            .get_pointer()
            .unwrap()
            .with_grab(|_, g| g.is::<NavigateGrab>())
            .unwrap_or(false);
        release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

        assert!(
            navigating,
            "{layer:?}: mod+ctrl+left must start center-nearest over a layer"
        );
        assert_eq!(
            layer_buttons(&mut f, id),
            Vec::new(),
            "{layer:?}: the grab consumed the click, so the layer must see no button"
        );
    }
}

/// A click held on a layer keeps routing the buttons pressed during it: an
/// `anywhere` action chorded onto it stays the layer's, press and release,
/// rather than firing and leaking its release through the held click.
#[test]
fn an_anywhere_action_chorded_onto_a_held_layer_click_stays_the_layers() {
    for layer in LAYERS {
        let mut f = Fixture::with_config(config(CLICK_ACTION));
        let id = pointer_on_layer(&mut f, layer);
        let resting = f.state().zoom_target();

        press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
        key_press(&mut f, KEY_LEFTMETA);
        press(&mut f, &FakeDevice::mouse(), BTN_RIGHT);
        release(&mut f, &FakeDevice::mouse(), BTN_RIGHT);
        release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

        assert_eq!(
            f.state().zoom_target(),
            resting,
            "{layer:?}: the held click owns the pointer, so mod+right must not fire"
        );
        assert_eq!(
            layer_buttons(&mut f, id),
            vec![
                (BTN_LEFT, PRESSED),
                (BTN_RIGHT, PRESSED),
                (BTN_RIGHT, RELEASED),
                (BTN_LEFT, RELEASED)
            ],
            "{layer:?}: every button of the chord is the layer's"
        );
    }
}

/// Below `interact_min` a bare scroll over a canvas window pans instead of
/// reaching the client. A bar over that window still gets its scroll.
#[test]
fn a_bare_scroll_over_a_bar_in_pick_mode_reaches_the_bar() {
    let mut f = Fixture::with_config(config(PICK_MODE));
    f.add_output(1, (1920, 1080));
    let bar = f.add_client();
    map_layer(&mut f, bar, Layer::Top);
    f.state().with_output_state(|os| os.zoom = 0.4);
    window_under(&mut f, cursor_on(Layer::Top));
    assert!(
        f.state().pick_mode(),
        "the zoom must sit below interact_min"
    );
    point_at_layer(&mut f, Layer::Top);
    let camera = f.state().camera();

    trackpad_scroll(&mut f, &FakeDevice::touchpad());

    assert_eq!(
        f.state().camera(),
        camera,
        "the pick-mode pan is for a window under the pointer, not one under a bar"
    );
    assert_eq!(
        layer_axes(&mut f, bar),
        vec![15.0],
        "the unbound scroll is the bar's"
    );
}

/// An unbound scroll over a stand-in is swallowed, since no client is there to
/// take it. A bar over that stand-in still gets its scroll.
#[test]
fn a_bare_scroll_over_a_bar_covering_a_stand_in_reaches_the_bar() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let bar = f.add_client();
    map_layer(&mut f, bar, Layer::Top);
    let centre = to_canvas(&mut f, cursor_on(Layer::Top));
    let origin = Point::from((centre.x as i32 - 200, centre.y as i32 - 150));
    let stand_in =
        f.state()
            .insert_suspended_for_test(1, origin, Size::from((400, 300)), "app", "App");
    point_at_layer(&mut f, Layer::Top);

    wheel_notch_down(&mut f, &FakeDevice::mouse());

    assert_eq!(
        layer_axes(&mut f, bar),
        vec![15.0],
        "the stand-in's swallow is for a scroll over it, not over a bar on top"
    );

    f.state().dismiss_suspended(stand_in);
}

/// A fullscreen window with an Overlay layer (the one layer drawn above it)
/// under the pointer. Returns the overlay's client.
fn overlay_over_fullscreen(f: &mut Fixture) -> ClientId {
    f.add_output(1, (1920, 1080));
    let app = f.add_client();
    let surface = map_window(f, app, "app", (400, 300));
    let window = window_by_app_id(f, "app").unwrap();
    let output = f.state().active_output().unwrap();
    f.state().enter_fullscreen(&window, Some(output));
    f.double_roundtrip(app);
    super::adopt_last_configure(f, app, &surface);
    assert!(f.state().is_fullscreen(), "the window must be fullscreen");

    let overlay = f.add_client();
    map_layer(f, overlay, Layer::Overlay);
    point_at_layer(f, Layer::Overlay);
    overlay
}

/// In fullscreen, bindings resolve on-window because the window fills the
/// screen, but not over an Overlay layer: there `alt+left` is the overlay's
/// click and must not exit fullscreen to start a move.
#[test]
fn an_on_window_binding_over_an_overlay_leaves_fullscreen_alone() {
    let mut f = Fixture::new();
    let overlay = overlay_over_fullscreen(&mut f);

    key_press(&mut f, KEY_LEFTALT);
    press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

    assert!(
        f.state().is_fullscreen(),
        "alt+left is an on-window binding, so it must not exit fullscreen over an overlay"
    );
    assert_eq!(
        layer_buttons(&mut f, overlay),
        vec![(BTN_LEFT, PRESSED), (BTN_LEFT, RELEASED)],
        "the unbound alt+left click is the overlay's"
    );
}

/// An `anywhere` zoom over an Overlay layer on a fullscreen output does what it
/// does over the fullscreen window: exit fullscreen and zoom.
#[test]
fn an_anywhere_zoom_over_an_overlay_exits_fullscreen_and_zooms() {
    let mut f = Fixture::new();
    let overlay = overlay_over_fullscreen(&mut f);
    let resting = f.state().zoom_target();

    key_press(&mut f, KEY_LEFTMETA);
    wheel_notch_down(&mut f, &FakeDevice::mouse());

    assert!(
        !f.state().is_fullscreen(),
        "mod+wheel zooms the canvas, which needs fullscreen gone"
    );
    assert_ne!(
        f.state().zoom_target(),
        resting,
        "mod+wheel must zoom over an overlay"
    );
    assert_eq!(
        layer_axes(&mut f, overlay),
        Vec::<f64>::new(),
        "the zoom consumed the scroll, so the overlay must see no axis"
    );
}

const ANYWHERE_MOVE: &str = r#"
[mouse.anywhere]
"mod+shift+left" = "move-window"
"#;

/// A window binding has no window to act on over a layer, even one bound in
/// `anywhere`, so over an Overlay it must not exit fullscreen for a move that
/// never starts.
#[test]
fn an_anywhere_window_binding_over_an_overlay_leaves_fullscreen_alone() {
    let mut f = Fixture::with_config(config(ANYWHERE_MOVE));
    let overlay = overlay_over_fullscreen(&mut f);

    key_press(&mut f, KEY_LEFTMETA);
    key_press(&mut f, KEY_LEFTSHIFT);
    press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

    assert!(
        f.state().is_fullscreen(),
        "a move binding over an overlay must not exit fullscreen"
    );
    assert_eq!(
        layer_buttons(&mut f, overlay),
        vec![(BTN_LEFT, PRESSED), (BTN_LEFT, RELEASED)],
        "the move has nothing to move over a layer, so the click is the overlay's"
    );
}

/// The fullscreen window claims every trigger below with `pass_mouse`. That is
/// a window rule, so over an Overlay layer it claims nothing.
const FULLSCREEN_CLAIMS: &str = r#"
[mouse.anywhere]
"mod+shift+wheel-down" = "toggle-fullscreen"

[[window_rules]]
app_id = "app"
pass_mouse = ["mod+left", "mod+wheel-scroll", "mod+shift+wheel-down"]
"#;

#[test]
fn pass_mouse_on_a_fullscreen_window_does_not_claim_a_click_over_an_overlay() {
    let mut f = Fixture::with_config(config(FULLSCREEN_CLAIMS));
    let overlay = overlay_over_fullscreen(&mut f);

    key_press(&mut f, KEY_LEFTMETA);
    press(&mut f, &FakeDevice::mouse(), BTN_LEFT);
    release(&mut f, &FakeDevice::mouse(), BTN_LEFT);

    assert!(
        !f.state().is_fullscreen(),
        "the mod+left pan must fire over the overlay, exiting fullscreen first"
    );
    assert_eq!(layer_buttons(&mut f, overlay), Vec::new());
}

#[test]
fn pass_mouse_on_a_fullscreen_window_does_not_claim_a_scroll_over_an_overlay() {
    let mut f = Fixture::with_config(config(FULLSCREEN_CLAIMS));
    let overlay = overlay_over_fullscreen(&mut f);

    key_press(&mut f, KEY_LEFTMETA);
    wheel_notch_down(&mut f, &FakeDevice::mouse());

    assert!(
        !f.state().is_fullscreen(),
        "the mod+wheel zoom must fire over the overlay, exiting fullscreen first"
    );
    assert_eq!(layer_axes(&mut f, overlay), Vec::<f64>::new());
}

#[test]
fn pass_mouse_on_a_fullscreen_window_does_not_claim_a_notch_over_an_overlay() {
    let mut f = Fixture::with_config(config(FULLSCREEN_CLAIMS));
    let overlay = overlay_over_fullscreen(&mut f);

    key_press(&mut f, KEY_LEFTMETA);
    key_press(&mut f, KEY_LEFTSHIFT);
    wheel_notch_down(&mut f, &FakeDevice::mouse());

    assert!(
        !f.state().is_fullscreen(),
        "the mod+shift+wheel-down toggle-fullscreen must fire over the overlay"
    );
    assert_eq!(layer_axes(&mut f, overlay), Vec::<f64>::new());
}
