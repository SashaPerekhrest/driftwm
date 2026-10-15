use smithay::{
    backend::input::{
        ButtonState, Device, DeviceCapability, Event, InputBackend, PointerButtonEvent,
        ProximityState, TabletToolButtonEvent, TabletToolEvent, TabletToolProximityEvent,
        TabletToolTipEvent, TabletToolTipState,
    },
    input::tablet::{TabletDescriptor, TabletSeatTrait, tool},
    output::Output,
    reexports::input::Device as LibinputDevice,
    utils::SERIAL_COUNTER,
};

use crate::input::touch::{as_libinput_device, is_internal_output, physical_size_matches};
use crate::state::{DriftWm, output_state};
use driftwm::canvas::{ScreenPos, screen_to_canvas};

/// Wrapper to route tablet tip events through the standard pointer button logic.
pub struct TabletTipButtonEvent<I: InputBackend> {
    event: I::TabletToolTipEvent,
    state: ButtonState,
}

impl<I: InputBackend> TabletTipButtonEvent<I> {
    pub fn new(event: I::TabletToolTipEvent, state: ButtonState) -> Self {
        Self { event, state }
    }
}

impl<I: InputBackend> Event<I> for TabletTipButtonEvent<I> {
    fn time(&self) -> u64 {
        <I::TabletToolTipEvent as Event<I>>::time(&self.event)
    }
    fn time_msec(&self) -> u32 {
        <I::TabletToolTipEvent as Event<I>>::time_msec(&self.event)
    }
    fn device(&self) -> <I as InputBackend>::Device {
        <I::TabletToolTipEvent as Event<I>>::device(&self.event)
    }
}

impl<I: InputBackend> PointerButtonEvent<I> for TabletTipButtonEvent<I> {
    fn button_code(&self) -> u32 {
        0x110 // BTN_LEFT
    }
    fn state(&self) -> ButtonState {
        self.state
    }
}

fn changed_axes<I: InputBackend>(event: &impl TabletToolEvent<I>) -> tool::AxisFrame {
    tool::AxisFrame {
        pressure: event.pressure_has_changed().then(|| event.pressure()),
        distance: event.distance_has_changed().then(|| event.distance()),
        tilt: event.tilt_has_changed().then(|| event.tilt()),
        rotation: event.rotation_has_changed().then(|| event.rotation()),
        slider: event.slider_has_changed().then(|| event.slider_position()),
        wheel: event
            .wheel_has_changed()
            .then(|| (event.wheel_delta(), event.wheel_delta_discrete())),
    }
}

impl DriftWm {
    pub fn on_device_added<I: InputBackend>(&mut self, device: &I::Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            let tablet_seat = self.seat.tablet_seat();
            let desc = TabletDescriptor::from(device);
            // `add_wp_tablet` replaces a known tablet, which clients see as an
            // unplug and a replug.
            if tablet_seat.get_tablet(&desc).is_none() {
                tablet_seat.add_wp_tablet(&self.display_handle, &desc);
            }
        }
    }

    pub fn on_device_removed<I: InputBackend>(&mut self, device: &I::Device) {
        if device.has_capability(DeviceCapability::TabletTool) {
            let tablet_seat = self.seat.tablet_seat();
            let desc = TabletDescriptor::from(device);
            tablet_seat.remove_tablet(&desc);
            if tablet_seat.count_tablets() == 0 {
                tablet_seat.clear_tools();
            }
        }
    }

    pub fn on_tablet_tool_axis<I: InputBackend>(&mut self, event: I::TabletToolAxisEvent)
    where
        I::Device: 'static,
    {
        let Some(output) = self.tablet_output_for_device::<I>(&event.device()) else {
            return;
        };
        let Some(output_geo) = self.space.output_geometry(&output) else {
            return;
        };

        let screen_pos = super::event_screen_pos::<I, _>(&output, output_geo.size, &event);

        let (camera, zoom) = {
            let os = output_state(&output);
            (os.camera, os.zoom)
        };
        let canvas_pos = screen_to_canvas(ScreenPos(screen_pos), camera, zoom).0;

        // The pen resolves its own output, so make that one active: everything
        // downstream reading `active_output()` — hot corners, later relative
        // motion — would otherwise act on whichever output the pointer was on.
        self.focused_output = Some(output.clone());
        // A pen is real pointer input, so it restores the cursor touch hid.
        self.cursor.hidden_by_touch = false;

        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();

        // Drives the seat pointer too, so menus and hover keep working in
        // clients that speak no tablet protocol; a client that does is
        // withheld from `wl_pointer` while the pen owns the pointer.
        self.pointer_pen = Some(event.tool());
        let under = self.dispatch_absolute_motion(&output, screen_pos, canvas_pos, serial, time);

        // Forward native tablet events to supporting clients
        let tablet_seat = self.seat.tablet_seat();
        if let Some(tool) = tablet_seat.get_tool(&event.tool()) {
            tool.axis(self, changed_axes(&event));
            tool.motion(
                self,
                under,
                &tool::MotionEvent {
                    location: canvas_pos,
                    serial,
                    time,
                },
            );
            tool.frame(self, time);
        }
    }

    pub fn on_tablet_tool_proximity<I: InputBackend>(&mut self, event: I::TabletToolProximityEvent)
    where
        I::Device: 'static,
    {
        let Some(output) = self.tablet_output_for_device::<I>(&event.device()) else {
            return;
        };
        let Some(output_geo) = self.space.output_geometry(&output) else {
            return;
        };

        let screen_pos = super::event_screen_pos::<I, _>(&output, output_geo.size, &event);

        let (camera, zoom) = {
            let os = output_state(&output);
            (os.camera, os.zoom)
        };
        let canvas_pos = screen_to_canvas(ScreenPos(screen_pos), camera, zoom).0;

        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();

        // The cascade below resolves layers, pins and pick mode through
        // `active_output()`, so the pen's own output has to be active before it
        // runs or a pinned tablet hit-tests its coordinates against another
        // monitor's scene.
        match event.state() {
            ProximityState::In => {
                self.focused_output = Some(output.clone());
                self.cursor.hidden_by_touch = false;
                self.pointer_pen = Some(event.tool());
            }
            ProximityState::Out => self.pointer_pen = None,
        }

        // Below `interact_min` a canvas window is a click target, not an input
        // surface, so the pen must not hand it tablet focus either. A later axis
        // event re-enters proximity through smithay's focus handling once the
        // zoom is back above it.
        let under = self.pointer_focus_under_pick(screen_pos, canvas_pos);

        let tablet_seat = self.seat.tablet_seat();
        let display_handle = self.display_handle.clone();
        let tool = tablet_seat
            .get_tool(&event.tool())
            .unwrap_or_else(|| tablet_seat.add_wp_tool(self, &display_handle, &event.tool()));
        match event.state() {
            ProximityState::In => {
                let Some(tablet) = tablet_seat.get_tablet(&TabletDescriptor::from(&event.device()))
                else {
                    return;
                };
                tool.proximity_in(
                    self,
                    under,
                    tablet,
                    &tool::ProximityInEvent {
                        location: canvas_pos,
                        axis: Some(changed_axes(&event)),
                        serial,
                        time,
                    },
                );
            }
            // Not gated on the tablet still being registered: the unplug that
            // removed it can precede this event, and the tool must still leave.
            ProximityState::Out => {
                tool.proximity_out(self, &tool::ProximityOutEvent { serial, time });
            }
        }
        tool.frame(self, time);
    }

    pub fn on_tablet_tool_tip<I: InputBackend>(&mut self, event: I::TabletToolTipEvent) {
        let tool = self.seat.tablet_seat().get_tool(&event.tool());
        // A tip carries no motion, so after a mouse moved the pointer the pen
        // has to reclaim it here, or the emulated press below reaches a
        // client that already has the tip on tablet-v2.
        self.pointer_pen = Some(event.tool());

        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();

        let button_state = match event.tip_state() {
            TabletToolTipState::Down => {
                if let Some(tool) = &tool {
                    tool.down(self, &tool::DownEvent { serial, time });
                }
                ButtonState::Pressed
            }
            TabletToolTipState::Up => {
                if let Some(tool) = &tool {
                    tool.up(self, &tool::UpEvent { serial, time });
                }
                ButtonState::Released
            }
        };
        if let Some(tool) = &tool {
            tool.frame(self, time);
        }

        // The press reuses the tip's serial: a client on tablet-v2 cites that
        // one to start a drag, which smithay accepts only against the pointer
        // grab this press installs.
        self.on_pointer_button::<I, _>(
            TabletTipButtonEvent::new(event, button_state),
            Some(serial),
        );
    }

    pub fn on_tablet_tool_button<I: InputBackend>(&mut self, event: I::TabletToolButtonEvent) {
        let tablet_seat = self.seat.tablet_seat();

        if let Some(tool) = tablet_seat.get_tool(&event.tool()) {
            let time = event.time_msec();
            tool.button(
                self,
                &tool::ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    button: event.button(),
                    state: event.button_state(),
                    time,
                },
            );
            tool.frame(self, time);
        }
    }

    /// Output a tablet from `device` maps to. Resolved per-device so multiple
    /// tablets each drive their own monitor. Resolution order: explicit
    /// config first, then libinput's output tag, then a single-output shortcut,
    /// then physical-size match (a digitizer is the same physical size as the
    /// panel it overlays), then the internal panel, then the first output.
    pub(crate) fn tablet_output_for_device<I: InputBackend>(
        &self,
        device: &I::Device,
    ) -> Option<Output>
    where
        I::Device: 'static,
    {
        if let Some(name) = self.config.tablet.map_to_output.as_deref()
            && let Some(o) = self.output_by_name(name)
        {
            return Some(o);
        }

        let libinput_device = as_libinput_device::<I>(device);

        if let Some(name) = libinput_device.and_then(LibinputDevice::output_name)
            && let Some(o) = self.output_by_name(&name)
        {
            return Some(o);
        }

        let mut outputs = self.space.outputs();
        let first = outputs.next().cloned();
        if outputs.next().is_none() {
            return first; // zero or one output: unambiguous
        }

        if let Some((dev_w, dev_h)) = libinput_device.and_then(LibinputDevice::size)
            && let Some(o) = self.space.outputs().find(|o| {
                let size = o.physical_properties().size;
                physical_size_matches(size.w as f64, size.h as f64, dev_w, dev_h)
            })
        {
            return Some(o.clone());
        }

        if let Some(o) = self.space.outputs().find(|o| is_internal_output(&o.name())) {
            return Some(o.clone());
        }

        first
    }
}
