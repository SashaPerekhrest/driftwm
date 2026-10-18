/// Bounding rectangle of a window in canvas coordinates, used for edge snap detection.
#[derive(Clone, Copy, Debug)]
pub struct SnapRect {
    pub x_low: f64,
    pub x_high: f64,
    pub y_low: f64,
    pub y_high: f64,
}

impl SnapRect {
    /// True when the two rects share positive-area overlap (touching edges
    /// don't count).
    pub fn overlaps(&self, other: &SnapRect) -> bool {
        self.x_low < other.x_high
            && other.x_low < self.x_high
            && self.y_low < other.y_high
            && other.y_low < self.y_high
    }

    /// The rect pulled in by `by` on all four sides; a negative `by` pushes
    /// it out.
    pub fn inset(&self, by: f64) -> SnapRect {
        SnapRect {
            x_low: self.x_low + by,
            x_high: self.x_high - by,
            y_low: self.y_low + by,
            y_high: self.y_high - by,
        }
    }
}

/// Parameters for snap candidate search along one axis.
pub struct SnapParams<'a> {
    pub extent: f64,
    pub perp_low: f64,
    pub perp_high: f64,
    pub horizontal: bool,
    pub others: &'a [SnapRect],
    pub gap: f64,
    pub threshold: f64,
    pub break_force: f64,
    pub same_edge: bool,
    pub edge_center: bool,
}

/// Per-axis snap state: tracks the snapped coordinate and the natural position
/// at the moment of engagement (used for directional break detection).
pub struct AxisSnap {
    pub snapped_pos: f64,
    pub natural_at_engage: f64,
}

/// Snap state for both axes plus cooldown after breaking a snap.
#[derive(Default)]
pub struct SnapState {
    pub x: Option<AxisSnap>,
    pub y: Option<AxisSnap>,
    pub cooldown_x: Option<f64>,
    pub cooldown_y: Option<f64>,
}

/// Try to beat the current best with a new candidate.
fn try_candidate(best: &mut Option<(f64, f64)>, snap_pos: f64, dist: f64, threshold: f64) {
    if dist < threshold && best.is_none_or(|(_, bd)| dist < bd) {
        *best = Some((snap_pos, dist));
    }
}

/// Which line of the primary a stop places: its low edge (left or top), its
/// high edge (right or bottom), or its midpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Low,
    High,
    Center,
}

/// How a stop relates the primary to the neighbour offering it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Facing edges a gap apart: the windows sit side by side, or corner to
    /// corner off a side dock.
    Dock,
    /// Same-side edges exactly aligned.
    SameEdge,
    /// Midpoints aligned along the axis.
    Center,
}

/// A coordinate along one axis that a neighbour offers one line of the primary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub line: Line,
    pub family: Family,
    /// Where the line lands.
    pub at: f64,
    /// Where the line meets the neighbour's edge, which is what a drag measures
    /// its distance from: a dock's contact sits the gap past `at`, an
    /// alignment's coincides with it.
    pub contact: f64,
}

/// The primary's side of a stop enumeration: which axis, the span it covers on
/// the other axis, and which families are on.
pub struct StopQuery {
    pub horizontal: bool,
    pub perp_low: f64,
    pub perp_high: f64,
    pub gap: f64,
    /// How far off the primary's perpendicular span a neighbour may sit and
    /// still offer alignments.
    pub align_band: f64,
    pub same_edge: bool,
    pub edge_center: bool,
    /// Whether a neighbour within `align_band` but clear of the perpendicular
    /// span offers the docks too, corner to corner: a keyboard move's way out
    /// of a column, which a drag leaves freely.
    pub side_docks: bool,
}

/// Every stop `others` offer along the query's axis, neighbour by neighbour.
///
/// A neighbour overlapping the primary's perpendicular span offers the docks;
/// one clear of it but within `align_band` offers the alignments instead, and
/// the docks too with `side_docks`. Never alignments to an overlapping one:
/// aligning to a window the primary already overlaps perpendicularly would
/// only z-stack the two.
pub fn stops<'a>(others: &'a [SnapRect], q: &'a StopQuery) -> impl Iterator<Item = Stop> + 'a {
    others.iter().flat_map(move |other| {
        let (other_low, other_high, other_perp_low, other_perp_high) = if q.horizontal {
            (other.x_low, other.x_high, other.y_low, other.y_high)
        } else {
            (other.y_low, other.y_high, other.x_low, other.x_high)
        };

        let perp_overlap = q.perp_high > other_perp_low && other_perp_high > q.perp_low;
        let aligns = !perp_overlap
            && perp_distance(q.perp_low, q.perp_high, other_perp_low, other_perp_high)
                < q.align_band;

        let docks = (perp_overlap || (aligns && q.side_docks)).then_some([
            Stop {
                line: Line::High,
                family: Family::Dock,
                at: other_low - q.gap,
                contact: other_low,
            },
            Stop {
                line: Line::Low,
                family: Family::Dock,
                at: other_high + q.gap,
                contact: other_high,
            },
        ]);
        let edges = (aligns && q.same_edge).then_some([
            Stop {
                line: Line::Low,
                family: Family::SameEdge,
                at: other_low,
                contact: other_low,
            },
            Stop {
                line: Line::High,
                family: Family::SameEdge,
                at: other_high,
                contact: other_high,
            },
        ]);
        let center = (aligns && q.edge_center).then(|| {
            let mid = (other_low + other_high) / 2.0;
            Stop {
                line: Line::Center,
                family: Family::Center,
                at: mid,
                contact: mid,
            }
        });
        docks
            .into_iter()
            .flatten()
            .chain(edges.into_iter().flatten())
            .chain(center)
    })
}

/// Offset from the primary's origin to `line`.
fn origin_shift(line: Line, extent: f64) -> f64 {
    match line {
        Line::Low => 0.0,
        Line::High => extent,
        Line::Center => extent / 2.0,
    }
}

impl SnapParams<'_> {
    fn stop_query(&self) -> StopQuery {
        StopQuery {
            horizontal: self.horizontal,
            perp_low: self.perp_low,
            perp_high: self.perp_high,
            gap: self.gap,
            // The band where a dock on the other axis would engage.
            align_band: self.gap + self.threshold,
            same_edge: self.same_edge,
            edge_center: self.edge_center,
            side_docks: false,
        }
    }
}

/// Find the best snap candidate along one axis: the closest of the [`stops`]
/// the neighbours offer, as the origin that puts the primary's line on it.
///
/// Returns `Some((snapped_origin, abs_distance))` for the closest candidate
/// within `threshold`, or `None`.
pub fn find_snap_candidate(natural_edge_low: f64, p: &SnapParams<'_>) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for stop in stops(p.others, &p.stop_query()) {
        let shift = origin_shift(stop.line, p.extent);
        try_candidate(
            &mut best,
            stop.at - shift,
            (natural_edge_low + shift - stop.contact).abs(),
            p.threshold,
        );
    }
    best
}

/// Half a pixel: a stop this close to a whole-pixel position is the one the
/// window already sits on, since every placement rounds to a pixel.
const ON_STOP: f64 = 0.5;

/// The nearest of `candidates` more than [`ON_STOP`] past `from`, toward
/// higher coordinates when `forward`.
fn nearest_beyond(
    from: f64,
    forward: bool,
    candidates: impl IntoIterator<Item = f64>,
) -> Option<f64> {
    candidates
        .into_iter()
        .filter(|&c| {
            if forward {
                c - from > ON_STOP
            } else {
                from - c > ON_STOP
            }
        })
        .reduce(|a, b| if forward { a.min(b) } else { a.max(b) })
}

/// The whole-pixel origin a whole-window move from `from` lands on at the next
/// stop in its direction, at any distance, from every [`stops`] family —
/// passing over any stop that would land it on one of `others`. A window it
/// already overlaps doesn't block it.
pub fn next_origin(
    from: f64,
    forward: bool,
    extent: f64,
    others: &[SnapRect],
    q: &StopQuery,
) -> Option<f64> {
    let frame_at = |origin: f64| {
        if q.horizontal {
            SnapRect {
                x_low: origin,
                x_high: origin + extent,
                y_low: q.perp_low,
                y_high: q.perp_high,
            }
        } else {
            SnapRect {
                x_low: q.perp_low,
                x_high: q.perp_high,
                y_low: origin,
                y_high: origin + extent,
            }
        }
    };
    let start = frame_at(from);
    let blockers: Vec<&SnapRect> = others.iter().filter(|o| !start.overlaps(o)).collect();
    // Rounded before the occupancy test, since that's where the window will
    // sit: a slot a fractional gap leaves half a pixel short takes it or not
    // depending on which way the half rounds.
    let clear = stops(others, q)
        .map(|s| (s.at - origin_shift(s.line, extent)).round())
        .filter(|&origin| {
            let landing = frame_at(origin);
            !blockers.iter().any(|b| landing.overlaps(b))
        });
    nearest_beyond(from, forward, clear)
}

/// The lines the primary's high edge can stop on: every one offered that
/// edge up to `bound`, and `bound` itself.
fn edge_lines<'a>(
    bound: f64,
    others: &'a [SnapRect],
    q: &'a StopQuery,
) -> impl Iterator<Item = f64> + 'a {
    stops(others, q)
        .filter(|s| s.line == Line::High)
        .map(|s| s.at)
        .filter(move |&at| at <= bound)
        .chain(std::iter::once(bound))
}

/// Where the primary's high edge at `edge` grows to next: the nearest stop
/// past `from` (`edge`, or further out when the walk is already heading there)
/// within `bound`, else `bound` itself — but never past the dock against the
/// nearest window ahead of `edge`, which growing on would cover.
pub fn next_grow_line(
    edge: f64,
    from: f64,
    bound: f64,
    others: &[SnapRect],
    q: &StopQuery,
) -> Option<f64> {
    // A window the frame already overlaps meets it behind `edge`, so it isn't
    // in the way.
    let dock = stops(others, q)
        .filter(|s| s.family == Family::Dock && s.line == Line::High)
        .filter(|s| s.contact > edge - ON_STOP)
        .map(|s| s.at)
        .reduce(f64::min);
    let reach = dock.map_or(bound, |dock| dock.min(bound));
    nearest_beyond(from, true, edge_lines(reach, others, q))
}

/// Where the primary's high edge at `edge` pulls back to next: the nearest
/// stop offered that edge within `bound` — or `bound` — still past `anchor`,
/// the low edge.
pub fn next_shrink_line(
    edge: f64,
    anchor: f64,
    bound: f64,
    others: &[SnapRect],
    q: &StopQuery,
) -> Option<f64> {
    let lines = edge_lines(bound, others, q).filter(|&at| at - anchor > ON_STOP);
    nearest_beyond(edge, false, lines)
}

/// Distance between two intervals `[lo1, hi1]` and `[lo2, hi2]`. Zero when
/// they overlap; otherwise the gap between the nearer endpoints.
fn perp_distance(lo1: f64, hi1: f64, lo2: f64, hi2: f64) -> f64 {
    if hi1 < lo2 {
        lo2 - hi1
    } else if hi2 < lo1 {
        lo1 - hi2
    } else {
        0.0
    }
}

/// Parameters for single-edge snap search (used during resize).
pub struct EdgeSnapParams<'a> {
    pub perp_low: f64,
    pub perp_high: f64,
    pub horizontal: bool,
    pub same_edge: bool,
    pub others: &'a [SnapRect],
    pub gap: f64,
    pub threshold: f64,
    pub break_force: f64,
    /// true = right/bottom edge, false = left/top edge.
    /// Controls gap direction: a high edge snaps to other_low with gap,
    /// a low edge snaps to other_high with gap.
    pub high_edge: bool,
}

/// Find the best snap target for a single edge (used during resize).
///
/// Unlike `find_snap_candidate` which snaps a whole window origin, this snaps
/// one active edge to the [`stops`] offered that edge alone.
/// Returns `Some((snapped_edge_pos, distance))`.
pub fn find_edge_snap(natural_edge: f64, p: &EdgeSnapParams<'_>) -> Option<(f64, f64)> {
    let q = StopQuery {
        horizontal: p.horizontal,
        perp_low: p.perp_low,
        perp_high: p.perp_high,
        gap: p.gap,
        align_band: p.gap + p.threshold,
        same_edge: p.same_edge,
        edge_center: false,
        side_docks: false,
    };
    let line = if p.high_edge { Line::High } else { Line::Low };
    let mut best: Option<(f64, f64)> = None;
    for stop in stops(p.others, &q).filter(|s| s.line == line) {
        try_candidate(
            &mut best,
            stop.at,
            (natural_edge - stop.contact).abs(),
            p.threshold,
        );
    }
    best
}

/// Update snap state for a single axis. Returns the final position for that axis.
pub fn update_axis(
    snap: &mut Option<AxisSnap>,
    cooldown: &mut Option<f64>,
    natural_pos: f64,
    p: &SnapParams<'_>,
) -> f64 {
    if let Some(ref s) = *snap {
        // Directional break: retreat past engagement point OR overshoot past snap
        let (retreat, overshoot) = if s.snapped_pos > s.natural_at_engage {
            (
                s.natural_at_engage - natural_pos,
                natural_pos - s.snapped_pos,
            )
        } else {
            (
                natural_pos - s.natural_at_engage,
                s.snapped_pos - natural_pos,
            )
        };
        if retreat >= p.break_force || overshoot >= p.break_force {
            *cooldown = Some(s.snapped_pos);
            *snap = None;
            natural_pos
        } else {
            s.snapped_pos
        }
    } else {
        // Clear cooldown when natural position leaves threshold of cooldown coord
        if let Some(cd) = *cooldown
            && (natural_pos - cd).abs() > p.threshold
        {
            *cooldown = None;
        }

        // Try to find a new snap candidate (skip if on cooldown)
        if cooldown.is_none()
            && let Some((snapped_pos, _)) = find_snap_candidate(natural_pos, p)
        {
            *snap = Some(AxisSnap {
                snapped_pos,
                natural_at_engage: natural_pos,
            });
            return snapped_pos;
        }

        natural_pos
    }
}

/// Apply edge snapping to an active resize operation.
///
/// Mutates `new_w`/`new_h` in place based on which edges are active.
/// `edges_mask` uses the xdg_toplevel resize edge bit layout (top=1, bottom=2, left=4, right=8).
///
/// The dragged window's *visible* edges (raw frame ± `self_bar` / `self_bw`)
/// are what get snapped against `others` — which are already inflated by their
/// own border width in `window_snap_rect`. Edges are inflated on the way in,
/// deflated on the way back to `new_w` / `new_h` (raw content size).
#[allow(clippy::too_many_arguments)]
pub fn snap_resize_edges(
    snap: &mut SnapState,
    edges_mask: u32,
    initial_location: (i32, i32),
    initial_size: (i32, i32),
    self_bar: i32,
    self_bw: i32,
    new_w: &mut i32,
    new_h: &mut i32,
    others: &[SnapRect],
    zoom: f64,
    gap: f64,
    snap_distance: f64,
    snap_break_force: f64,
    same_edge: bool,
) {
    let effective_distance = snap_distance / zoom;
    let effective_break = snap_break_force / zoom;
    let (loc_x, loc_y) = (initial_location.0 as f64, initial_location.1 as f64);
    let (init_w, init_h) = (initial_size.0 as f64, initial_size.1 as f64);
    let bw = self_bw as f64;

    let has_top = edges_mask & 1 != 0;
    let has_bottom = edges_mask & 2 != 0;
    let has_left = edges_mask & 4 != 0;
    let has_right = edges_mask & 8 != 0;

    // When a Y edge is already held-snapped, use the snapped visible position
    // instead of the natural (cursor-driven) one. Otherwise break_force drift
    // in the natural height could let the X-edge snap engage against a target
    // the window doesn't visually overlap — spurious corner snap.
    // `snap.y.snapped_pos` stores the visible (inflated) edge after the call
    // below, so we use it directly without further adjustment.
    let visual_top = if has_top {
        snap.y
            .as_ref()
            .map_or(loc_y + init_h - *new_h as f64 - self_bar as f64 - bw, |s| {
                s.snapped_pos
            })
    } else {
        loc_y - self_bar as f64 - bw
    };
    let visual_bottom = if has_bottom {
        snap.y
            .as_ref()
            .map_or(loc_y + *new_h as f64 + bw, |s| s.snapped_pos)
    } else {
        loc_y + init_h + bw
    };

    if has_right {
        let natural_right = loc_x + *new_w as f64 + bw;
        let hp = EdgeSnapParams {
            perp_low: visual_top,
            perp_high: visual_bottom,
            horizontal: true,
            same_edge,
            others,
            gap,
            threshold: effective_distance,
            break_force: effective_break,
            high_edge: true,
        };
        let snapped = update_edge(&mut snap.x, &mut snap.cooldown_x, natural_right, &hp);
        *new_w = (snapped - loc_x - bw) as i32;
    } else if has_left {
        let fixed_right = loc_x + init_w;
        let natural_left = fixed_right - *new_w as f64 - bw;
        let hp = EdgeSnapParams {
            perp_low: visual_top,
            perp_high: visual_bottom,
            horizontal: true,
            same_edge,
            others,
            gap,
            threshold: effective_distance,
            break_force: effective_break,
            high_edge: false,
        };
        let snapped = update_edge(&mut snap.x, &mut snap.cooldown_x, natural_left, &hp);
        *new_w = (fixed_right - snapped - bw) as i32;
    }

    // Visible X range for the Y-edge snap's perpendicular check. The X block
    // above has already updated *new_w to reflect any X snap, so we can derive
    // the visible range from that. A left-edge resize anchors to the right
    // side (fixed_right), so its visible X range is NOT (loc_x, loc_x + new_w).
    let (x_perp_low, x_perp_high) = if has_left {
        (loc_x + init_w - *new_w as f64 - bw, loc_x + init_w + bw)
    } else if has_right {
        (loc_x - bw, loc_x + *new_w as f64 + bw)
    } else {
        (loc_x - bw, loc_x + init_w + bw)
    };

    if has_bottom {
        let natural_bottom = loc_y + *new_h as f64 + bw;
        let vp = EdgeSnapParams {
            perp_low: x_perp_low,
            perp_high: x_perp_high,
            horizontal: false,
            same_edge,
            others,
            gap,
            threshold: effective_distance,
            break_force: effective_break,
            high_edge: true,
        };
        let snapped = update_edge(&mut snap.y, &mut snap.cooldown_y, natural_bottom, &vp);
        *new_h = (snapped - loc_y - bw) as i32;
    } else if has_top {
        let fixed_bottom = loc_y + init_h;
        let natural_top = fixed_bottom - *new_h as f64 - self_bar as f64 - bw;
        let vp = EdgeSnapParams {
            perp_low: x_perp_low,
            perp_high: x_perp_high,
            horizontal: false,
            same_edge,
            others,
            gap,
            threshold: effective_distance,
            break_force: effective_break,
            high_edge: false,
        };
        let snapped = update_edge(&mut snap.y, &mut snap.cooldown_y, natural_top, &vp);
        *new_h = (fixed_bottom - snapped - self_bar as f64 - bw) as i32;
    }

    *new_w = (*new_w).max(1);
    *new_h = (*new_h).max(1);
}

/// Update snap state for a single edge during resize. Returns the final edge position.
pub fn update_edge(
    snap: &mut Option<AxisSnap>,
    cooldown: &mut Option<f64>,
    natural_edge: f64,
    p: &EdgeSnapParams<'_>,
) -> f64 {
    if let Some(ref s) = *snap {
        let (retreat, overshoot) = if s.snapped_pos > s.natural_at_engage {
            (
                s.natural_at_engage - natural_edge,
                natural_edge - s.snapped_pos,
            )
        } else {
            (
                natural_edge - s.natural_at_engage,
                s.snapped_pos - natural_edge,
            )
        };
        if retreat >= p.break_force || overshoot >= p.break_force {
            *cooldown = Some(s.snapped_pos);
            *snap = None;
            natural_edge
        } else {
            s.snapped_pos
        }
    } else {
        if let Some(cd) = *cooldown
            && (natural_edge - cd).abs() > p.threshold
        {
            *cooldown = None;
        }

        if cooldown.is_none()
            && let Some((snapped_pos, _)) = find_edge_snap(natural_edge, p)
        {
            *snap = Some(AxisSnap {
                snapped_pos,
                natural_at_engage: natural_edge,
            });
            return snapped_pos;
        }

        natural_edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn rect(x_low: f64, y_low: f64, w: f64, h: f64) -> SnapRect {
        SnapRect {
            x_low,
            x_high: x_low + w,
            y_low,
            y_high: y_low + h,
        }
    }

    /// Perpendicular span `[0, 100]`, every family on, the default gap and band.
    fn query(horizontal: bool) -> StopQuery {
        StopQuery {
            horizontal,
            perp_low: 0.0,
            perp_high: 100.0,
            gap: 12.0,
            align_band: 36.0,
            same_edge: true,
            edge_center: true,
            side_docks: false,
        }
    }

    fn lines(stops: impl Iterator<Item = Stop>) -> Vec<(Line, Family, f64)> {
        stops.map(|s| (s.line, s.family, s.at)).collect()
    }

    #[test]
    fn a_perpendicular_overlap_offers_the_two_docks_and_nothing_else() {
        let others = [rect(500.0, 50.0, 200.0, 100.0)];
        assert_eq!(
            lines(stops(&others, &query(true))),
            vec![
                (Line::High, Family::Dock, 488.0),
                (Line::Low, Family::Dock, 712.0)
            ]
        );
        let contacts: Vec<f64> = stops(&others, &query(true)).map(|s| s.contact).collect();
        assert_eq!(contacts, vec![500.0, 700.0], "a dock measures edge to edge");
    }

    #[test]
    fn a_perpendicular_adjacent_neighbour_offers_the_three_alignments() {
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        assert_eq!(
            lines(stops(&others, &query(true))),
            vec![
                (Line::Low, Family::SameEdge, 100.0),
                (Line::High, Family::SameEdge, 400.0),
                (Line::Center, Family::Center, 250.0)
            ]
        );
    }

    #[test]
    fn alignments_switch_off_with_their_flags() {
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        let neither = StopQuery {
            same_edge: false,
            edge_center: false,
            ..query(true)
        };
        assert_eq!(stops(&others, &neither).count(), 0);
        let edges_only = StopQuery {
            edge_center: false,
            ..query(true)
        };
        assert_eq!(stops(&others, &edges_only).count(), 2);
    }

    #[test]
    fn side_docks_give_an_adjacent_neighbour_the_docks_as_well() {
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        let q = StopQuery {
            side_docks: true,
            ..query(true)
        };
        assert_eq!(
            lines(stops(&others, &q)),
            vec![
                (Line::High, Family::Dock, 88.0),
                (Line::Low, Family::Dock, 412.0),
                (Line::Low, Family::SameEdge, 100.0),
                (Line::High, Family::SameEdge, 400.0),
                (Line::Center, Family::Center, 250.0)
            ]
        );
        let contacts: Vec<f64> = stops(&others, &q).take(2).map(|s| s.contact).collect();
        assert_eq!(contacts, vec![100.0, 400.0], "measured from its edges");
        assert_eq!(
            stops(&[rect(100.0, 136.0, 300.0, 100.0)], &q).count(),
            0,
            "past the band, nothing"
        );
    }

    #[test]
    fn a_neighbour_at_the_band_offers_nothing() {
        let others = [rect(100.0, 136.0, 300.0, 100.0)];
        assert_eq!(stops(&others, &query(true)).count(), 0);
    }

    #[test]
    fn the_vertical_axis_reads_the_rect_transposed() {
        let others = [rect(50.0, 500.0, 100.0, 200.0)];
        assert_eq!(
            lines(stops(&others, &query(false))),
            vec![
                (Line::High, Family::Dock, 488.0),
                (Line::Low, Family::Dock, 712.0)
            ]
        );
    }

    #[test]
    fn a_move_walks_to_the_nearest_stop_past_its_origin() {
        // Extent 200: the high dock at 488 puts the origin at 288, the low
        // dock at 712 leaves it there.
        let others = [rect(500.0, 50.0, 200.0, 100.0)];
        let q = query(true);
        assert_eq!(next_origin(0.0, true, 200.0, &others, &q), Some(288.0));
        assert_eq!(next_origin(288.0, true, 200.0, &others, &q), Some(712.0));
        assert_eq!(next_origin(712.0, true, 200.0, &others, &q), None);
        assert_eq!(next_origin(712.0, false, 200.0, &others, &q), Some(288.0));
        assert_eq!(next_origin(288.0, false, 200.0, &others, &q), None);
    }

    #[test]
    fn a_stop_the_window_rounded_onto_does_not_count() {
        // A 12.5 gap puts the dock at 287.5, which the move rounds to 288;
        // from 288 that stop is neither ahead nor behind.
        let others = [rect(500.0, 50.0, 200.0, 100.0)];
        let q = StopQuery {
            gap: 12.5,
            ..query(true)
        };
        assert_eq!(next_origin(0.0, true, 200.0, &others, &q), Some(288.0));
        assert_eq!(next_origin(288.0, true, 200.0, &others, &q), Some(713.0));
        assert_eq!(next_origin(288.0, false, 200.0, &others, &q), None);
    }

    #[test]
    fn a_move_takes_alignments_from_an_adjacent_neighbour() {
        // Left edges at 100, centres at 150, right edges at 200.
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        let q = query(true);
        assert_eq!(next_origin(0.0, true, 200.0, &others, &q), Some(100.0));
        assert_eq!(next_origin(100.0, true, 200.0, &others, &q), Some(150.0));
        assert_eq!(next_origin(150.0, true, 200.0, &others, &q), Some(200.0));
        assert_eq!(next_origin(200.0, true, 200.0, &others, &q), None);
    }

    #[test]
    fn a_move_with_side_docks_steps_out_a_gap_past_an_adjacent_neighbour() {
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        let q = StopQuery {
            side_docks: true,
            ..query(true)
        };
        assert_eq!(next_origin(100.0, false, 200.0, &others, &q), Some(-112.0));
        assert_eq!(next_origin(-112.0, true, 200.0, &others, &q), Some(100.0));
        assert_eq!(next_origin(200.0, true, 200.0, &others, &q), Some(412.0));
        assert_eq!(next_origin(412.0, true, 200.0, &others, &q), None);
    }

    #[test]
    fn a_move_passes_over_a_side_dock_that_lands_on_a_window() {
        let others = [
            rect(100.0, 112.0, 300.0, 100.0),
            // Docked left of the primary, in the slot the side dock offers.
            rect(-112.0, 0.0, 200.0, 100.0),
        ];
        let q = StopQuery {
            side_docks: true,
            ..query(true)
        };
        assert_eq!(next_origin(100.0, false, 200.0, &others, &q), Some(-324.0));
    }

    #[test]
    fn a_drag_offers_no_side_dock() {
        let others = [rect(100.0, 112.0, 300.0, 100.0)];
        let p = SnapParams {
            extent: 200.0,
            perp_low: 0.0,
            perp_high: 100.0,
            horizontal: true,
            others: &others,
            gap: 12.0,
            threshold: 24.0,
            break_force: 32.0,
            same_edge: true,
            edge_center: true,
        };
        // Right on the spots a move's side docks put the primary.
        assert_eq!(find_snap_candidate(-112.0, &p), None);
        assert_eq!(find_snap_candidate(412.0, &p), None);
    }

    #[test]
    fn a_move_along_y_walks_the_transposed_stops() {
        let others = [rect(50.0, 500.0, 100.0, 200.0)];
        assert_eq!(
            next_origin(0.0, true, 100.0, &others, &query(false)),
            Some(388.0)
        );
    }

    #[test]
    fn a_move_passes_over_stops_that_land_on_a_window() {
        // A docked row of 200-wide windows: the docks either side of the
        // middle one are the outer ones' slots.
        let others = [
            rect(212.0, 0.0, 200.0, 100.0),
            rect(424.0, 0.0, 200.0, 100.0),
        ];
        let q = query(true);
        assert_eq!(next_origin(0.0, true, 200.0, &others, &q), Some(636.0));
        assert_eq!(next_origin(636.0, false, 200.0, &others, &q), Some(0.0));
    }

    #[test]
    fn a_move_along_y_passes_over_a_window_too() {
        let others = [
            rect(0.0, 212.0, 100.0, 200.0),
            rect(0.0, 424.0, 100.0, 200.0),
        ];
        assert_eq!(
            next_origin(0.0, true, 200.0, &others, &query(false)),
            Some(636.0)
        );
    }

    #[test]
    fn a_move_is_not_blocked_by_a_window_it_already_overlaps() {
        let others = [
            rect(100.0, 50.0, 500.0, 100.0),
            // Below the primary: its edges and centre all align at 300.
            rect(300.0, 112.0, 200.0, 100.0),
        ];
        assert_eq!(
            next_origin(0.0, true, 200.0, &others, &query(true)),
            Some(300.0),
            "still over the wide neighbour, which it was over already"
        );
    }

    #[test]
    fn a_slot_half_a_pixel_short_takes_the_window_only_where_it_rounds_clear() {
        // A 12.5 gap leaves the slot between the two windows half a pixel
        // short: docked before the second, the window starts at 199.5 and
        // rounds clear of the first; docked after the first, at 212.5, it
        // rounds a whole pixel onto the second.
        let others = [rect(0.0, 0.0, 200.0, 100.0), rect(412.0, 0.0, 200.0, 100.0)];
        let q = StopQuery {
            gap: 12.5,
            ..query(true)
        };
        assert_eq!(next_origin(-213.0, true, 200.0, &others, &q), Some(200.0));
        assert_eq!(next_origin(200.0, true, 200.0, &others, &q), Some(625.0));
    }

    #[test]
    fn a_grow_walks_neighbour_lines_up_to_the_dock_and_stops_there() {
        let others = [
            rect(600.0, 50.0, 200.0, 100.0),
            // Below the primary, offering its right edge as an alignment.
            rect(0.0, 112.0, 500.0, 100.0),
        ];
        let q = query(true);
        assert_eq!(
            next_grow_line(400.0, 400.0, 1000.0, &others, &q),
            Some(500.0)
        );
        assert_eq!(
            next_grow_line(500.0, 500.0, 1000.0, &others, &q),
            Some(588.0)
        );
        assert_eq!(
            next_grow_line(588.0, 588.0, 1000.0, &others, &q),
            None,
            "growing on would cover the neighbour"
        );
    }

    #[test]
    fn a_grow_closer_than_the_gap_to_a_neighbour_goes_nowhere() {
        let others = [rect(600.0, 50.0, 200.0, 100.0)];
        assert_eq!(
            next_grow_line(595.0, 595.0, 1000.0, &others, &query(true)),
            None
        );
    }

    #[test]
    fn a_grow_walking_on_past_its_edge_still_stops_at_a_window_its_frame_meets() {
        // The frame ends at 685 but the walk is already heading for 700; the
        // window docked at 697 sits in between.
        let others = [
            rect(697.0, 50.0, 400.0, 100.0),
            rect(0.0, 112.0, 700.0, 100.0),
        ];
        assert_eq!(
            next_grow_line(685.0, 700.0, 1820.0, &others, &query(true)),
            None
        );
    }

    #[test]
    fn a_grow_passes_a_window_it_already_overlaps_and_docks_at_the_next() {
        let others = [
            rect(300.0, 50.0, 200.0, 100.0),
            rect(700.0, 50.0, 200.0, 100.0),
        ];
        assert_eq!(
            next_grow_line(400.0, 400.0, 1000.0, &others, &query(true)),
            Some(688.0)
        );
    }

    #[test]
    fn a_grow_ignores_a_neighbour_line_past_the_bound() {
        let others = [rect(1100.0, 50.0, 200.0, 100.0)];
        assert_eq!(
            next_grow_line(400.0, 400.0, 1000.0, &others, &query(true)),
            Some(1000.0)
        );
    }

    #[test]
    fn a_grow_with_no_neighbour_reaches_the_bound() {
        assert_eq!(
            next_grow_line(400.0, 400.0, 1000.0, &[], &query(true)),
            Some(1000.0)
        );
    }

    #[test]
    fn a_shrink_walks_back_through_the_lines_inside_the_span() {
        let others = [rect(600.0, 50.0, 200.0, 100.0)];
        let q = query(true);
        assert_eq!(
            next_shrink_line(1000.0, 0.0, 1000.0, &others, &q),
            Some(588.0)
        );
        assert_eq!(next_shrink_line(588.0, 0.0, 1000.0, &others, &q), None);
    }

    #[test]
    fn a_shrink_past_the_bound_pulls_back_to_it_first() {
        let others = [rect(600.0, 50.0, 200.0, 100.0)];
        assert_eq!(
            next_shrink_line(1200.0, 0.0, 1000.0, &others, &query(true)),
            Some(1000.0)
        );
    }

    #[test]
    fn a_shrink_past_the_bound_skips_lines_past_it_too() {
        // Below the primary, its right edge at 1100 is off the usable area.
        let others = [rect(0.0, 112.0, 1100.0, 100.0)];
        assert_eq!(
            next_shrink_line(1200.0, 0.0, 1000.0, &others, &query(true)),
            Some(1000.0)
        );
    }

    #[test]
    fn a_shrink_never_closes_the_span_onto_its_anchor() {
        let others = [rect(600.0, 50.0, 200.0, 100.0)];
        assert_eq!(
            next_shrink_line(1000.0, 588.0, 1000.0, &others, &query(true)),
            None
        );
    }

    fn any_rect() -> impl Strategy<Value = SnapRect> {
        (
            -1000.0..1000.0f64,
            -1000.0..1000.0f64,
            20.0..600.0f64,
            20.0..600.0f64,
        )
            .prop_map(|(x, y, w, h)| rect(x, y, w, h))
    }

    proptest! {
        /// The walk's ordering: it lands on an enumerated stop strictly
        /// beyond its origin, clear of every window it wasn't already over,
        /// and skips no such stop on the way. The stops' own coordinates are
        /// pinned by the table cases above.
        #[test]
        fn next_origin_takes_the_nearest_clear_stop_beyond_and_skips_none(
            from in -1000.0..1000.0f64,
            forward in any::<bool>(),
            extent in 20.0..600.0f64,
            others in prop::collection::vec(any_rect(), 0..6),
            side_docks in any::<bool>(),
        ) {
            let q = StopQuery { side_docks, ..query(true) };
            let origins: Vec<f64> = stops(&others, &q)
                .map(|s| (s.at - origin_shift(s.line, extent)).round())
                .collect();
            let frame = |o: f64| rect(o, q.perp_low, extent, q.perp_high - q.perp_low);
            let lands_on_a_window = |o: f64| {
                others.iter().any(|n| frame(o).overlaps(n) && !frame(from).overlaps(n))
            };
            let open = |o: f64| {
                let beyond = if forward { o - from > ON_STOP } else { from - o > ON_STOP };
                beyond && !lands_on_a_window(o)
            };
            match next_origin(from, forward, extent, &others, &q) {
                Some(o) => {
                    prop_assert!(open(o));
                    prop_assert!(origins.contains(&o));
                    let none_skipped = origins
                        .iter()
                        .all(|&c| !open(c) || if forward { c >= o } else { c <= o });
                    prop_assert!(none_skipped);
                }
                None => prop_assert!(origins.iter().all(|&c| !open(c))),
            }
        }

        /// A grow never covers a window the frame wasn't already over, even
        /// when the walk is already heading past its edge, and never passes
        /// the bound.
        #[test]
        fn a_grow_never_covers_a_window(
            anchor in -1000.0..1000.0f64,
            width in 20.0..600.0f64,
            ahead in 0.0..40.0f64,
            bound in -1000.0..2000.0f64,
            others in prop::collection::vec(any_rect(), 0..6),
        ) {
            let q = query(true);
            let edge = anchor + width;
            let from = edge + ahead;
            let frame = |high: f64| rect(anchor, q.perp_low, high - anchor, q.perp_high - q.perp_low).inset(ON_STOP);
            if let Some(line) = next_grow_line(edge, from, bound, &others, &q) {
                prop_assert!(line - from > ON_STOP);
                prop_assert!(line <= bound);
                let covered = others
                    .iter()
                    .any(|n| frame(line).overlaps(n) && !frame(edge).overlaps(n));
                prop_assert!(!covered);
            }
        }
    }
}
