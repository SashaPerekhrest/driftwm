//! `move-to-snap` / `resize-to-snap`: the keyboard walks between the stops
//! drag snapping offers — every family, at any distance, whatever the
//! `[snap] corners` / `centers` magnetism knobs say — instead of stepping by
//! a fixed number of pixels.

use std::collections::HashSet;

use smithay::utils::Point;

use super::{DriftWm, NavZoom};
use driftwm::config::Direction;
use driftwm::layout::snap::{self, Line, SnapRect, StopQuery};

impl DriftWm {
    /// Move the focused canvas element along `dir` to the nearest stop past
    /// where it sits, and bring the camera along if that leaves the viewport.
    /// Nothing that way is a no-op.
    pub(crate) fn move_to_snap_focused(&mut self, dir: &Direction) {
        let Some((horizontal, forward)) = axis_of(dir) else {
            return;
        };
        let Some(element) = self.focused_element().filter(|e| self.is_canvas_window(e)) else {
            return;
        };
        let Some(loc) = self.stage.position_of(&element) else {
            return;
        };

        #[allow(clippy::mutable_key_type)]
        let excludes = HashSet::new();
        let (others, bar, bw) = self.snap_targets(&element, &excludes);
        let frame = super::fit::snap_rect_at(loc, element.geometry().size, bar, bw);
        let q = self.stop_query(&frame, horizontal);
        let (from, extent) = if horizontal {
            (frame.x_low, frame.x_high - frame.x_low)
        } else {
            (frame.y_low, frame.y_high - frame.y_low)
        };
        let Some(origin) = snap::next_origin(from, forward, extent, &others, &q) else {
            return;
        };
        // The stop places the visual frame; the stage holds the content
        // top-left, one border in and the bar down from it.
        let new_loc = if horizontal {
            Point::from(((origin + bw as f64).round() as i32, loc.y))
        } else {
            Point::from((loc.x, (origin + (bar + bw) as f64).round() as i32))
        };

        // A nudge's bookkeeping: the move re-anchors the element, so any fill
        // restore point and owed recenter are stale, and its canvas position
        // is durable session state.
        self.stage.clear_fill(&element);
        self.drop_owed_recenter(&element);
        self.map_window(element.clone(), new_loc, false);
        self.animate_element_move_from(&element, loc, None);
        self.session_store_mark_dirty();

        if !self.window_fully_in_viewport(&element) {
            self.navigate_to_element(&element, NavZoom::Keep);
        }
    }

    /// Resize the focused canvas element by walking its right or bottom edge:
    /// `right` / `down` grow it out to the next stop, `left` / `up` pull it
    /// back to the previous one, never past the usable area.
    pub(crate) fn resize_to_snap_focused(&mut self, dir: &Direction) {
        let Some((horizontal, grow)) = axis_of(dir) else {
            return;
        };
        let Some((element, start)) = self.step_target() else {
            return;
        };
        let chrome = self.element_chrome(&element);
        let frame = super::fit::snap_rect_at(start.loc, start.size, chrome.bar, chrome.border);
        #[allow(clippy::mutable_key_type)]
        let excludes = HashSet::new();
        let (others, _, _) = self.snap_targets(&element, &excludes);
        let Some(output) = self.output_for_window(&element) else {
            return;
        };
        // The edge `fill-window` grows to on that side.
        let bounds = self.usable_canvas_bounds(&output);
        let inset = self.config.outer_frame_inset(chrome.border);
        let q = self.stop_query(&frame, horizontal);
        let (edge, anchor, bound) = if horizontal {
            (frame.x_high, frame.x_low, bounds.x_high - inset)
        } else {
            (frame.y_high, frame.y_low, bounds.y_high - inset)
        };
        let target = if grow {
            snap::next_grow_line(edge, Line::High, bound, &others, &q)
        } else {
            snap::next_shrink_line(edge, Line::High, anchor, bound, &others, &q)
        };
        let Some(target) = target else {
            return;
        };
        let axis = if horizontal {
            Direction::Right
        } else {
            Direction::Down
        };
        self.step_resize(&element, start, &axis, (target - edge).round() as i32);
    }

    /// The query for `frame`'s stops along one axis: every family on, the
    /// alignment band a drag at zoom 1 would use.
    fn stop_query(&self, frame: &SnapRect, horizontal: bool) -> StopQuery {
        let (perp_low, perp_high) = if horizontal {
            (frame.y_low, frame.y_high)
        } else {
            (frame.x_low, frame.x_high)
        };
        StopQuery {
            horizontal,
            perp_low,
            perp_high,
            gap: self.config.snap_gap,
            align_band: self.config.snap_gap + self.config.snap_distance,
            same_edge: true,
            edge_center: true,
        }
    }
}

/// `(horizontal, toward higher coordinates)` for a cardinal direction. The
/// diagonals never parse for these actions.
fn axis_of(dir: &Direction) -> Option<(bool, bool)> {
    match dir {
        Direction::Left => Some((true, false)),
        Direction::Right => Some((true, true)),
        Direction::Up => Some((false, false)),
        Direction::Down => Some((false, true)),
        _ => None,
    }
}
