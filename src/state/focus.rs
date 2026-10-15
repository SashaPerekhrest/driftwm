//! FocusTarget newtype — the SeatHandler focus type for keyboard, pointer, and touch.
//!
//! Required because `PopupGrab` needs `KeyboardFocus: From<PopupKind>`, and we
//! can't impl `From<PopupKind> for WlSurface` (orphan rule). All input-target
//! methods delegate to the inner `WlSurface`.

use std::borrow::Cow;

use smithay::{
    backend::input::{KeyState, TabletToolDescriptor},
    desktop::PopupKind,
    input::{
        Seat, SeatHandler,
        dnd::{DndFocus, Source},
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, CursorImageStatus, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
        tablet::{
            self, Tablet, TabletSeatTrait,
            tool::{TabletToolHandle, TabletToolTarget},
        },
        touch::{
            DownEvent as TouchDownEvent, FrameMarker, MotionEvent as TouchMotionEvent, TouchTarget,
            UpEvent as TouchUpEvent,
        },
    },
    reexports::wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Logical, Point, Serial},
    wayland::seat::WaylandFocus,
};

use crate::state::DriftWm;

// --- FocusTarget ---
// Newtype over WlSurface for use as SeatHandler focus types.
// Required because PopupGrab needs `KeyboardFocus: From<PopupKind>`,
// and we can't impl `From<PopupKind> for WlSurface` (orphan rule).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusTarget(pub WlSurface);

impl FocusTarget {
    /// Whether this surface's client has `tool` bound. Such a client takes
    /// the pen on tablet-v2, and `wl_pointer` is not its stream for it.
    pub fn takes_tool(&self, tool: &TabletToolHandle<DriftWm>) -> bool {
        self.0
            .client()
            .is_some_and(|client| tool.client_tools(&client).next().is_some())
    }
}

/// Window-level keyboard-focus intent: either a real surface, or a suspended
/// window (which holds no seat keyboard focus — the actual focus derives to
/// `None` while a suspended window is the intended target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusIntent {
    Surface(FocusTarget),
    Suspended(crate::state::SuspendedId),
}

impl From<PopupKind> for FocusTarget {
    fn from(popup: PopupKind) -> Self {
        FocusTarget(popup.wl_surface().clone())
    }
}

impl IsAlive for FocusTarget {
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl WaylandFocus for FocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        Some(Cow::Borrowed(&self.0))
    }
}

// Delegate all KeyboardTarget methods to the inner WlSurface using
// fully-qualified syntax to avoid clash with WlSurface::enter() protocol method.
impl KeyboardTarget<DriftWm> for FocusTarget {
    fn enter(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        <WlSurface as KeyboardTarget<DriftWm>>::enter(&self.0, seat, data, keys, serial);
    }

    fn leave(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, serial: Serial) {
        <WlSurface as KeyboardTarget<DriftWm>>::leave(&self.0, seat, data, serial);
    }

    fn key(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        <WlSurface as KeyboardTarget<DriftWm>>::key(&self.0, seat, data, key, state, serial, time);
    }

    fn modifiers(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        <WlSurface as KeyboardTarget<DriftWm>>::modifiers(&self.0, seat, data, modifiers, serial);
    }
}

impl PointerTarget<DriftWm> for FocusTarget {
    fn enter(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &MotionEvent) {
        <WlSurface as PointerTarget<DriftWm>>::enter(&self.0, seat, data, event);
    }

    fn motion(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &MotionEvent) {
        <WlSurface as PointerTarget<DriftWm>>::motion(&self.0, seat, data, event);
    }

    fn relative_motion(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &RelativeMotionEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::relative_motion(&self.0, seat, data, event);
    }

    fn button(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &ButtonEvent) {
        <WlSurface as PointerTarget<DriftWm>>::button(&self.0, seat, data, event);
    }

    fn axis(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, frame: AxisFrame) {
        <WlSurface as PointerTarget<DriftWm>>::axis(&self.0, seat, data, frame);
    }

    fn frame(&self, seat: &Seat<DriftWm>, data: &mut DriftWm) {
        <WlSurface as PointerTarget<DriftWm>>::frame(&self.0, seat, data);
    }

    fn gesture_swipe_begin(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GestureSwipeBeginEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_swipe_begin(&self.0, seat, data, event);
    }

    fn gesture_swipe_update(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GestureSwipeUpdateEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_swipe_update(&self.0, seat, data, event);
    }

    fn gesture_swipe_end(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GestureSwipeEndEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_swipe_end(&self.0, seat, data, event);
    }

    fn gesture_pinch_begin(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GesturePinchBeginEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_pinch_begin(&self.0, seat, data, event);
    }

    fn gesture_pinch_update(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GesturePinchUpdateEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_pinch_update(&self.0, seat, data, event);
    }

    fn gesture_pinch_end(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GesturePinchEndEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_pinch_end(&self.0, seat, data, event);
    }

    fn gesture_hold_begin(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GestureHoldBeginEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_hold_begin(&self.0, seat, data, event);
    }

    fn gesture_hold_end(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &GestureHoldEndEvent,
    ) {
        <WlSurface as PointerTarget<DriftWm>>::gesture_hold_end(&self.0, seat, data, event);
    }

    fn leave(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, serial: Serial, time: u32) {
        <WlSurface as PointerTarget<DriftWm>>::leave(&self.0, seat, data, serial, time);
    }
}

impl TouchTarget<DriftWm> for FocusTarget {
    fn down(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &TouchDownEvent) {
        data.touch_state.frame_owed = true;
        <WlSurface as TouchTarget<DriftWm>>::down(&self.0, seat, data, event);
    }

    fn up(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &TouchUpEvent) {
        data.touch_state.frame_owed = true;
        <WlSurface as TouchTarget<DriftWm>>::up(&self.0, seat, data, event);
    }

    fn motion(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, event: &TouchMotionEvent) {
        data.touch_state.frame_owed = true;
        <WlSurface as TouchTarget<DriftWm>>::motion(&self.0, seat, data, event);
    }

    fn frame(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, marker: FrameMarker) {
        <WlSurface as TouchTarget<DriftWm>>::frame(&self.0, seat, data, marker);
    }

    fn cancel(&self, seat: &Seat<DriftWm>, data: &mut DriftWm, marker: FrameMarker) {
        <WlSurface as TouchTarget<DriftWm>>::cancel(&self.0, seat, data, marker);
    }

    fn shape(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &smithay::input::touch::ShapeEvent,
    ) {
        data.touch_state.frame_owed = true;
        <WlSurface as TouchTarget<DriftWm>>::shape(&self.0, seat, data, event);
    }

    fn orientation(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        event: &smithay::input::touch::OrientationEvent,
    ) {
        data.touch_state.frame_owed = true;
        <WlSurface as TouchTarget<DriftWm>>::orientation(&self.0, seat, data, event);
    }

    fn last_frame(&self, seat: &Seat<DriftWm>, data: &mut DriftWm) -> Option<FrameMarker> {
        <WlSurface as TouchTarget<DriftWm>>::last_frame(&self.0, seat, data)
    }
}

impl TabletToolTarget<DriftWm> for FocusTarget {
    fn proximity_in(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        tablet: &Tablet,
        serial: Serial,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::proximity_in(
            &self.0,
            seat,
            data,
            tool_descriptor,
            tablet,
            serial,
        );
    }

    fn proximity_out(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::proximity_out(
            &self.0,
            seat,
            data,
            tool_descriptor,
        );
        // smithay drops a client's cursor on the `wl_pointer` leave, which a
        // client withheld from `wl_pointer` never gets; the image its tool
        // set — hidden, for a drawing canvas — would outlive the pen's visit.
        // A client without the tool still holds `wl_pointer` and its own
        // cursor, which no pull would set again.
        if seat
            .tablet_seat()
            .get_tool(tool_descriptor)
            .is_some_and(|tool| self.takes_tool(&tool))
        {
            data.cursor_image(seat, CursorImageStatus::default_named());
        }
    }

    fn down(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        event: &tablet::tool::DownEvent,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::down(&self.0, seat, data, tool_descriptor, event);
    }

    fn up(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        event: &tablet::tool::UpEvent,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::up(&self.0, seat, data, tool_descriptor, event);
    }

    fn motion(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        event: &tablet::tool::MotionEvent,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::motion(
            &self.0,
            seat,
            data,
            tool_descriptor,
            event,
        );
    }

    fn axis(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        frame: tablet::tool::AxisFrame,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::axis(&self.0, seat, data, tool_descriptor, frame);
    }

    fn button(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        event: &tablet::tool::ButtonEvent,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::button(
            &self.0,
            seat,
            data,
            tool_descriptor,
            event,
        );
    }

    fn frame(
        &self,
        seat: &Seat<DriftWm>,
        data: &mut DriftWm,
        tool_descriptor: &TabletToolDescriptor,
        time: u32,
    ) {
        <WlSurface as TabletToolTarget<DriftWm>>::frame(&self.0, seat, data, tool_descriptor, time);
    }
}

impl<D> DndFocus<D> for FocusTarget
where
    D: SeatHandler + smithay::wayland::selection::data_device::DataDeviceHandler + 'static,
    WlSurface: DndFocus<D>,
{
    type OfferData<S>
        = <WlSurface as DndFocus<D>>::OfferData<S>
    where
        S: Source;

    fn enter<S: Source>(
        &self,
        data: &mut D,
        dh: &DisplayHandle,
        source: std::sync::Arc<S>,
        seat: &Seat<D>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        <WlSurface as DndFocus<D>>::enter(&self.0, data, dh, source, seat, location, serial)
    }

    fn motion<S: Source>(
        &self,
        data: &mut D,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<D>,
        location: Point<f64, Logical>,
        time: u32,
    ) {
        <WlSurface as DndFocus<D>>::motion(&self.0, data, offer, seat, location, time)
    }

    fn leave<S: Source>(
        &self,
        data: &mut D,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<D>,
    ) {
        <WlSurface as DndFocus<D>>::leave(&self.0, data, offer, seat)
    }

    fn drop<S: Source>(
        &self,
        data: &mut D,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<D>,
    ) {
        <WlSurface as DndFocus<D>>::drop(&self.0, data, offer, seat)
    }
}
