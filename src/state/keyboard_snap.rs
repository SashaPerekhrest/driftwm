//! `move-to-snap` / `resize-to-snap`: the keyboard walks between the stops
//! drag snapping offers, every family at any distance.

use std::collections::HashSet;

use smithay::utils::Point;

use super::{DriftWm, NavZoom};
use driftwm::config::Direction;
use driftwm::layout::snap::{self, SnapRect, StopQuery};

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
        // The size the placement assumes, not committed geometry: the action
        // prelude may have just exited fullscreen, and the client goes on
        // reporting the output's size until it acks the restore.
        let size = self.placed_element_size(&element);
        let frame = super::fit::snap_rect_at(loc, size, bar, bw);
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

        // The move re-anchors the element: its fill restore point and any owed
        // recenter are stale, and its position is session state.
        self.stage.clear_fill(&element);
        self.drop_owed_recenter(&element);
        self.map_window(element.clone(), new_loc, false);
        self.animate_element_move_from(&element, loc, None);
        self.session_store_mark_dirty();

        // A pan still in flight is heading for a view picked for the last
        // stop, wherever its current frame shows. `camera_target` alone can't
        // tell: the zoom anchor every navigation pan rides clears it each tick.
        let at_rest = self.camera_target().is_none() && self.zoom_target().is_none();
        let landed = super::fit::snap_rect_at(new_loc, size, bar, bw);
        if !at_rest || !self.frame_fully_in_view_at(&landed, self.camera(), self.zoom()) {
            self.navigate_to_element(&element, NavZoom::Keep);
        }
    }

    /// Resize the focused canvas element by walking its right or bottom edge:
    /// `right` / `down` grow it out to the next stop, `left` / `up` pull it
    /// back to the previous one, never past the usable area or over another
    /// window.
    pub(crate) fn resize_to_snap_focused(&mut self, dir: &Direction) {
        let Some((horizontal, grow)) = axis_of(dir) else {
            return;
        };
        let Some((element, start)) = self.step_target() else {
            return;
        };
        #[allow(clippy::mutable_key_type)]
        let excludes = HashSet::new();
        let (others, bar, bw) = self.snap_targets(&element, &excludes);
        let placed = super::fit::snap_rect_at(start.loc, start.size, bar, bw);
        // A client that rounds to a cell answers short of, or past, the stop
        // it was asked for. Walking on from that answer would find the same
        // stop and re-ask for it on every press, so walk from the request too.
        let requested =
            super::fit::snap_rect_at(start.loc, self.requested_element_size(&element), bar, bw);
        let Some(output) = self.output_for_window(&element) else {
            return;
        };
        // The fence `fill-window` grows inside, refused the same way for a
        // window lying entirely outside it: `output_for_window` falls back to
        // the active output for a window no viewport shows, and that
        // viewport's far edge is no bound for a window screens away.
        let fence = self
            .usable_canvas_bounds(&output)
            .inset(self.config.outer_frame_inset(bw));
        if !requested.overlaps(&fence) {
            return;
        }
        let q = self.stop_query(&requested, horizontal);
        let (placed_edge, requested_edge, anchor, bound) = if horizontal {
            (placed.x_high, requested.x_high, placed.x_low, fence.x_high)
        } else {
            (placed.y_high, requested.y_high, placed.y_low, fence.y_high)
        };
        // Beyond both, so a stop the rounding happened to land on counts as
        // reached rather than becoming a step of nothing.
        let target = if grow {
            let edge = placed_edge.max(requested_edge);
            snap::next_grow_line(edge, bound, &others, &q)
        } else {
            let edge = placed_edge.min(requested_edge);
            snap::next_shrink_line(edge, anchor, bound, &others, &q)
        };
        let Some(target) = target else {
            return;
        };
        let axis = if horizontal {
            Direction::Right
        } else {
            Direction::Down
        };
        // `step_resize` adds the step to the placed size.
        let step = (target - placed_edge).round() as i32;
        self.step_resize(&element, start, &axis, step);
    }

    /// The query for `frame`'s stops along one axis, with the alignment band a
    /// drag at zoom 1 would use.
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
            // The magnetism knobs tune a drag, not what the keyboard can reach.
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
