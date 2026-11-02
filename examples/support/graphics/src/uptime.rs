//! Service health presentation. Every interval and total comes from the workload;
//! this module only places queried and arrival-order state on matching axes.
use crate::{
    labels::{Labels, Style},
    uptime_report::{Input, Report, Status, Timeline},
};
use bevy::{math::Isometry2d, prelude::*};

const BRIGHT: Color = Color::srgb(0.78, 0.87, 0.90);
const PENDING_OPACITY: f32 = 0.70;
const MUTED: Color = Color::srgb(0.56, 0.67, 0.73);
const HISTORY_LEFT: f32 = -72.0;
const HISTORY_WIDTH: f32 = 128.0;
#[path = "uptime_transfers.rs"]
mod transfers;
use transfers::Transfers;
#[path = "uptime_playback.rs"]
mod playback;
pub(crate) use playback::Playback;

#[derive(Clone, Copy)]
struct Axis {
    start: i128,
    end: i128,
    left: f32,
    width: f32,
}
impl Axis {
    fn x(self, time: u64) -> f32 {
        self.left + self.width * ((i128::from(time) - self.start) as f64 / (self.end - self.start).max(1) as f64).clamp(0.0, 1.0) as f32
    }
    fn segment(self, start: u64, end: u64) -> Option<(f32, f32)> {
        let start = i128::from(start).max(self.start);
        let end = i128::from(end).min(self.end);
        (end > start).then(|| (self.x(start as u64), self.x(end as u64)))
    }
}
fn health(status: Status) -> Color {
    match status {
        Status::Healthy => Color::srgb_u8(84, 205, 139),
        Status::Degraded => Color::srgb_u8(244, 184, 67),
        Status::Down => Color::srgb_u8(239, 95, 104),
    }
}
fn identity(id: usize, alpha: f32) -> Color {
    // The default five are distinct categories, not neighboring hues.
    // Red, yellow and green remain reserved for health status.
    const COLORS: [[u8; 3]; 5] = [
        [99, 206, 219],  // soft cyan #63CEDB
        [229, 128, 191], // soft magenta #E580BF
        [162, 138, 229], // soft violet #A28AE5
        [190, 148, 120], // light cocoa #BE9478
        [213, 212, 199], // ivory #D5D4C7
    ];
    let slot = id.saturating_sub(1) % crate::uptime_report::MAX_SERVICES;
    let [red, green, blue] = COLORS[slot % COLORS.len()];
    // Larger configured runs retain unique darker shades of the same safe palette.
    let shade = 1.0 - (slot / COLORS.len()) as f32 * 0.18;
    Color::srgba(red as f32 / 255.0 * shade, green as f32 / 255.0 * shade, blue as f32 / 255.0 * shade, alpha)
}
fn name(status: Status) -> &'static str {
    match status {
        Status::Healthy => "Healthy",
        Status::Degraded => "Degraded",
        Status::Down => "Down",
    }
}
#[derive(Clone, Copy)]
struct Row {
    center: f32,
    reconciled: f32,
    arrival: f32,
    height: f32,
}
fn row(index: usize, count: usize) -> Row {
    let step = (78.0 / count.max(1) as f32).min(18.0);
    let center = 30.0 - index as f32 * step;
    Row { center, reconciled: center + step * 0.2, arrival: center - step * 0.2, height: (step * 0.14).clamp(0.5, 1.4) }
}
fn history_axis_at(milliseconds: u64) -> Axis {
    // A fixed ten-second scale avoids stretching all history during startup.
    let end = i128::from(milliseconds);
    Axis { start: end - 10_000, end, left: HISTORY_LEFT, width: HISTORY_WIDTH }
}
fn history_axis(report: &Report) -> Axis {
    history_axis_at(report.now_ms)
}
fn block(gizmos: &mut Gizmos, left: f32, right: f32, y: f32, height: f32, color: Color) {
    // Closely spaced strokes form a solid health strip at all supported scales.
    let strokes = (height / 0.18).ceil() as usize;
    for index in 0..=strokes {
        let yy = y - height * 0.5 + height * index as f32 / strokes.max(1) as f32;
        gizmos.line_2d(Vec2::new(left, yy), Vec2::new(right, yy), color);
    }
}
fn histories(report: &Report, viewport_ms: u64, labels: &mut Labels, gizmos: &mut Gizmos) {
    let axis = history_axis_at(viewport_ms);
    labels.responsive(
        Vec2::new(-100.0, 43.0),
        200.0,
        "WITH CONTIME ABOVE / WITHOUT RECONCILIATION BELOW",
        "CONTIME ABOVE / NO REPLAY BELOW",
        Style::Heading,
        BRIGHT,
    );
    for (index, service) in report.services.iter().enumerate() {
        let row = row(index, report.services.len());
        let color = identity(service.id, 1.0);
        gizmos.circle_2d(Vec2::new(-100.0, row.center), 0.5, color).resolution(12);
        labels.push(Vec2::new(-98.0, row.center), 25.0, &service.name, Style::Legend, color);
        for (y, timeline) in [(row.reconciled, &service.reconciled), (row.arrival, &service.arrival_order)] {
            // Both strips use this exact time axis, so discrepancies line up vertically.
            gizmos.rect_2d(
                Isometry2d::from_translation(Vec2::new(axis.left + axis.width / 2.0, y)),
                Vec2::new(axis.width, row.height + 0.25),
                MUTED.with_alpha(0.25),
            );
            intervals(timeline, axis, y, row.height, gizmos);
            if report.services.len() <= 8 {
                for ((status, milliseconds), x) in summary_totals(timeline).into_iter().zip([59.0, 75.0, 91.0]) {
                    symbol(gizmos, Vec2::new(x, y), status, 1.0, health(status));
                    labels.push(Vec2::new(x + 2.0, y), 13.0, &seconds(milliseconds), Style::Legend, BRIGHT);
                }
            }
        }
    }
    if !report.services.is_empty() {
        let first = row(0, report.services.len());
        let last = row(report.services.len() - 1, report.services.len());
        grid(gizmos, axis, last.arrival - 2.0, first.reconciled + 2.0);
    }
    // Markers sit above the grid so the grid cannot cut through their outlines.
    for (index, service) in report.services.iter().enumerate() {
        let row = row(index, report.services.len());
        let size = (row.height * 0.95).clamp(0.45, 1.4);
        for (y, timeline) in [(row.reconciled, &service.reconciled), (row.arrival, &service.arrival_order)] {
            for (x, status) in transition_marks(timeline, axis) {
                symbol(gizmos, Vec2::new(x, y), status, size, health(status));
            }
        }
    }
}
fn summary_totals(timeline: &Timeline) -> [(Status, u64); 3] {
    // Backend uptime includes degraded availability. Present the three icon
    // states separately so the totals do not double-count degraded time.
    [
        (Status::Healthy, timeline.uptime_ms.saturating_sub(timeline.degraded_ms)),
        (Status::Down, timeline.downtime_ms),
        (Status::Degraded, timeline.degraded_ms),
    ]
}
fn seconds(milliseconds: u64) -> String {
    let seconds = milliseconds as f64 / 1000.0;
    if seconds < 1000.0 {
        format!("{seconds:.2}s")
    } else if seconds < 1_000_000.0 {
        format!("{seconds:.0}s")
    } else {
        format!("{seconds:.0e}s")
    }
}
/// Absolute simulation seconds align every service and both comparison rows.
fn second_ticks(axis: Axis) -> impl Iterator<Item = (u64, f32)> {
    let first = (axis.start.max(0) + 999) / 1000 * 1000;
    (first..=axis.end).step_by(1000).map(move |time| (time as u64, axis.x(time as u64)))
}
fn grid(gizmos: &mut Gizmos, axis: Axis, bottom: f32, top: f32) {
    for (_, x) in second_ticks(axis) {
        gizmos.line_2d(Vec2::new(x, bottom), Vec2::new(x, top), MUTED.with_alpha(0.24));
    }
}
fn transition_marks(timeline: &Timeline, axis: Axis) -> impl Iterator<Item = (f32, Status)> + '_ {
    timeline.intervals.iter().filter_map(move |interval| {
        let start = i128::from(interval.start_ms);
        // A retained window starts in an existing state; that clipped edge is not
        // a new health change. Keep only genuine visible transition boundaries.
        (start >= axis.start && start <= axis.end && (start > axis.start || start == 0))
            .then(|| (axis.x(interval.start_ms), interval.status))
    })
}
fn history_segments(timeline: &Timeline, axis: Axis) -> impl Iterator<Item = (f32, f32, Status)> + '_ {
    timeline.intervals.iter().enumerate().filter_map(move |(index, interval)| {
        // Hold the visible current status to the scrolling edge between queries.
        // Only a landed report can change its color; totals remain queried values.
        let end = if index + 1 == timeline.intervals.len() { interval.end_ms.max(axis.end.max(0) as u64) } else { interval.end_ms };
        axis.segment(interval.start_ms, end).map(|(start, end)| (start, end, interval.status))
    })
}
fn intervals(timeline: &Timeline, axis: Axis, y: f32, height: f32, gizmos: &mut Gizmos) {
    for (start, end, status) in history_segments(timeline, axis) {
        block(gizmos, start, end, y, height, health(status));
    }
}
#[path = "uptime_tracks.rs"]
mod tracks;
use tracks::Tracks;
pub(crate) struct NetworkLayout {
    axis: Axis,
    positions: Vec<Vec2>,
    spacing: f32,
}
impl NetworkLayout {
    #[cfg(test)]
    fn new(report: &Report) -> Self {
        let mut tracks = Tracks::default();
        tracks.update(report);
        Self::with_tracks(report, &tracks)
    }
    fn with_tracks(report: &Report, tracks: &Tracks) -> Self {
        let spacing = 24.0 / report.services.len().max(1) as f32;
        let axis = network_axis(report);
        let positions = report
            .inputs
            .iter()
            .map(|input| {
                let index = report.services.iter().position(|service| service.id == input.service).unwrap_or(0);
                Vec2::new(
                    axis.x(input.arrived_ms.unwrap_or(input.delivery_ms)),
                    76.0 - index as f32 * spacing + tracks.offset(input.id) * spacing,
                )
            })
            .collect();
        Self { axis, positions, spacing }
    }
    fn row_y(&self, index: usize) -> f32 {
        76.0 - index as f32 * self.spacing
    }
}
fn network_axis(report: &Report) -> Axis {
    // Fixed scale and signed origin keep DELIVERY at 65%, including startup.
    // Long pending delays are clipped rather than changing the scale mid-run.
    Axis { start: i128::from(report.clock_ms) - 6_500, end: i128::from(report.clock_ms) + 3_500, left: -96.0, width: 192.0 }
}
#[derive(Debug, PartialEq)]
struct InputGeometry {
    delay: Option<(f32, f32)>,
    action: Option<f32>,
    delivery: Option<f32>,
    immediate: bool,
}
fn input_geometry(input: &Input, axis: Axis) -> InputGeometry {
    let end = input.arrived_ms.unwrap_or(input.delivery_ms);
    let visible = |time| (axis.start..=axis.end).contains(&i128::from(time)).then(|| axis.x(time));
    InputGeometry {
        delay: axis.segment(input.original_ms, end),
        action: visible(input.original_ms),
        delivery: visible(end),
        immediate: end == input.original_ms,
    }
}
fn network(report: &Report, layout: &NetworkLayout, labels: &mut Labels, gizmos: &mut Gizmos) {
    let axis = layout.axis;
    let delivery = axis.x(report.clock_ms);
    grid(gizmos, axis, 51.0, 78.0);
    labels.push(Vec2::new(-100.0, 85.0), 150.0, "SERVICE HEALTH / INPUT TIMELINE", Style::Heading, BRIGHT);
    labels.push(Vec2::new((delivery - 4.0).min(78.0), 79.5), 30.0, "DELIVERY", Style::Legend, BRIGHT);
    gizmos.line_2d(Vec2::new(delivery, 78.0), Vec2::new(delivery, 51.0), BRIGHT);
    for (index, service) in report.services.iter().enumerate() {
        let y = layout.row_y(index);
        gizmos.circle_2d(Vec2::new(-100.0, y), 0.5, identity(service.id, 1.0)).resolution(12);
        gizmos.line_2d(Vec2::new(-96.0, y), Vec2::new(96.0, y), MUTED.with_alpha(0.15));
        for (i, input) in report.inputs.iter().enumerate().filter(|(_, input)| input.service == service.id) {
            let y = layout.positions[i].y;
            let geometry = input_geometry(input, axis);
            let color = identity(service.id, if input.arrived_ms.is_some() { 0.85 } else { PENDING_OPACITY });
            if let Some((start, stop)) = geometry.delay {
                gizmos.line_2d(Vec2::new(start, y), Vec2::new(stop, y), color);
            }
            if let Some(x) = geometry.action {
                symbol(gizmos, Vec2::new(x, y), input.status, (layout.spacing * 0.25).clamp(0.3, 1.3), color);
            }
            if let Some(x) = geometry.delivery.filter(|_| !geometry.immediate) {
                crate::timeline::delivery_tick(gizmos, Vec2::new(x, y), 0.4, input.late, color);
            }
        }
    }
    labels.push(Vec2::new(-100.0, 48.0), 200.0, "Check: healthy / !: degraded / X: down / Double tick: late", Style::Hint, MUTED);
}
fn symbol(gizmos: &mut Gizmos, p: Vec2, status: Status, size: f32, color: Color) {
    let segments = match status {
        Status::Healthy => [([-0.85, 0.0], [-0.25, -0.65]), ([-0.25, -0.65], [0.85, 0.8])],
        Status::Degraded => [([0.0, 0.9], [0.0, -0.25]), ([-0.12, -0.75], [0.12, -0.75])],
        Status::Down => [([-0.75, -0.75], [0.75, 0.75]), ([-0.75, 0.75], [0.75, -0.75])],
    };
    // Stroke the outline first, then the entire colored glyph. This preserves
    // its health/service color without cutting a rectangular gap in the strip.
    let outline = Color::srgb_u8(4, 8, 12);
    let radius = (size * 0.22).clamp(0.12, 0.30);
    for direction in 0..8 {
        let offset = Vec2::from_angle(direction as f32 * std::f32::consts::TAU / 8.0) * radius;
        for (a, b) in segments {
            gizmos.line_2d(p + Vec2::from_array(a) * size + offset, p + Vec2::from_array(b) * size + offset, outline);
        }
    }
    for (a, b) in segments {
        gizmos.line_2d(p + Vec2::from_array(a) * size, p + Vec2::from_array(b) * size, color);
    }
}

pub(crate) fn draw(
    report: &Report,
    visible: &Report,
    layout: &NetworkLayout,
    viewport_ms: u64,
    state: (bool, bool),
    labels: &mut Labels,
    gizmos: &mut Gizmos,
) {
    let (running, finished) = state;
    network(report, layout, labels, gizmos);
    histories(visible, viewport_ms, labels, gizmos);
    for (x, status) in [(-100.0, Status::Healthy), (-65.0, Status::Degraded), (-25.0, Status::Down)] {
        symbol(gizmos, Vec2::new(x + 1.5, -50.0), status, 0.9, health(status));
        labels.push(Vec2::new(x + 4.0, -49.5), 32.0, name(status), Style::Legend, MUTED);
    }
    labels.push(
        Vec2::new(-100.0, -55.0),
        200.0,
        "Grid: one second / Paired timelines share timestamps / Totals cover the whole run",
        Style::Hint,
        MUTED,
    );
    let state = if finished {
        "FINISHED"
    } else if !running {
        "PAUSED"
    } else {
        ""
    };
    labels.push(Vec2::new(60.0, 91.0), 42.0, state, Style::Heading, BRIGHT);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_symbols_mark_status_changes_not_clipped_window_edges() {
        use crate::uptime_report::Interval;
        let timeline = Timeline {
            intervals: vec![
                Interval { start_ms: 100, end_ms: 200, status: Status::Healthy },
                Interval { start_ms: 200, end_ms: 300, status: Status::Down },
                Interval { start_ms: 300, end_ms: 400, status: Status::Degraded },
                Interval { start_ms: 400, end_ms: 500, status: Status::Healthy },
            ],
            ..Default::default()
        };
        let axis = Axis { start: 100, end: 450, left: -10.0, width: 20.0 };
        assert_eq!(
            transition_marks(&timeline, axis).collect::<Vec<_>>(),
            vec![(axis.x(200), Status::Down), (axis.x(300), Status::Degraded), (axis.x(400), Status::Healthy)]
        );
        let clipped = Axis { start: 250, ..axis };
        assert_eq!(transition_marks(&timeline, clipped).count(), 2);
    }
    #[test]
    fn summary_states_are_disjoint_whole_run_totals_not_visible_window_durations() {
        let timeline = Timeline { uptime_ms: 8000, downtime_ms: 2000, degraded_ms: 1000, ..Default::default() };
        assert_eq!(summary_totals(&timeline), [(Status::Healthy, 7000), (Status::Down, 2000), (Status::Degraded, 1000)]);
        assert_eq!(seconds(1250), "1.25s");
        assert_eq!(seconds(1_250_000), "1250s");
        assert!(seconds(u64::MAX).contains('e'), "very long runs must keep summaries compact");
    }
    #[test]
    fn grid_uses_absolute_seconds_in_both_views_and_clips_startup() {
        let history = history_axis_at(12_350);
        let network = network_axis(&Report { clock_ms: 12_350, ..Default::default() });
        for axis in [history, network, history_axis_at(1500)] {
            let ticks: Vec<_> = second_ticks(axis).collect();
            assert!(!ticks.is_empty());
            for &(time, x) in &ticks {
                assert_eq!(time % 1000, 0);
                assert!(i128::from(time) >= axis.start && i128::from(time) <= axis.end);
                assert_eq!(x, axis.x(time));
            }
            for pair in ticks.windows(2) {
                assert_eq!(pair[1].0 - pair[0].0, 1000);
                assert!((pair[1].1 - pair[0].1 - axis.width / 10.0).abs() < 0.0001);
            }
        }
        assert_eq!(second_ticks(history_axis_at(1500)).map(|(time, _)| time).collect::<Vec<_>>(), vec![0, 1000]);
    }
    #[test]
    fn history_scale_is_fixed_from_startup_and_scrolls_without_stretching() {
        let start = history_axis_at(1000);
        let later = history_axis_at(1016);
        assert_eq!(start.end - start.start, 10_000);
        assert_eq!(later.end - later.start, 10_000);
        let movement = start.x(500) - later.x(500);
        assert!((movement - HISTORY_WIDTH * 16.0 / 10_000.0).abs() < 0.0001);
    }
    #[test]
    fn scrolling_holds_only_the_visible_status_without_changing_queried_data() {
        use crate::uptime_report::Interval;
        let timeline = Timeline {
            status: Status::Down,
            downtime_ms: 50,
            intervals: vec![Interval { start_ms: 1000, end_ms: 1050, status: Status::Down }],
            ..Default::default()
        };
        let axis = history_axis_at(1066);
        let segments = history_segments(&timeline, axis).collect::<Vec<_>>();
        assert_eq!(segments, vec![(axis.x(1000), axis.x(1066), Status::Down)]);
        assert_eq!(timeline.intervals[0].end_ms, 1050);
        assert_eq!(timeline.downtime_ms, 50);
    }
    #[test]
    fn paired_histories_share_the_same_time_axis_and_clip_intervals() {
        let report = Report { now_ms: 200, window_start_ms: 100, ..Default::default() };
        let axis = Axis { start: report.window_start_ms.into(), end: report.now_ms.into(), left: HISTORY_LEFT, width: HISTORY_WIDTH };
        assert_eq!(axis.segment(0, 150), Some((HISTORY_LEFT, HISTORY_LEFT + HISTORY_WIDTH / 2.0)));
        assert_eq!(axis.segment(150, 300), Some((HISTORY_LEFT + HISTORY_WIDTH / 2.0, HISTORY_LEFT + HISTORY_WIDTH)));
        assert_eq!(axis.segment(0, 99), None);
        assert_eq!(axis.segment(160, 150), None);
        assert!(Axis { end: 100, ..axis }.x(100).is_finite());
    }
    #[test]
    fn delivery_clock_stays_live_while_queries_are_behind() {
        let report = Report { clock_ms: 20_000, now_ms: 12_000, window_start_ms: 2_000, ..Default::default() };
        let axis = network_axis(&report);
        let input = Input {
            id: 1,
            service: 1,
            status: Status::Down,
            original_ms: 18_000,
            delivery_ms: 19_000,
            arrived_ms: Some(19_500),
            late: false,
        };
        assert_eq!(axis.start, 13_500);
        let geometry = input_geometry(&input, axis);
        assert!(geometry.delivery.unwrap() <= axis.x(report.clock_ms));
        assert!(geometry.delivery.unwrap() > axis.x(report.now_ms));
        // Service histories keep their own validated query horizon.
        assert_eq!(report.now_ms, 12_000);
    }
    #[test]
    fn delivery_marker_is_fixed_at_sixty_five_percent_even_at_startup() {
        for clock_ms in [0, 100, 20_000] {
            let mut report = Report { clock_ms, ..Default::default() };
            let axis = network_axis(&report);
            assert!(((axis.x(clock_ms) - axis.left) / axis.width - 0.65).abs() < 0.0001);
            report.inputs.push(Input {
                id: 1,
                service: 1,
                status: Status::Down,
                original_ms: clock_ms,
                delivery_ms: clock_ms + 100_000,
                arrived_ms: None,
                late: false,
            });
            let pending = network_axis(&report);
            assert_eq!(pending.x(clock_ms), axis.x(clock_ms));
            assert_eq!(pending.end - pending.start, 10_000);
        }
    }
    #[test]
    fn expired_inputs_have_no_delay_action_or_delivery_glyphs() {
        let axis = Axis { start: 10_000, end: 21_000, left: -96.0, width: 192.0 };
        let mut input =
            Input { id: 1, service: 1, status: Status::Healthy, original_ms: 100, delivery_ms: 200, arrived_ms: Some(300), late: true };
        assert_eq!(input_geometry(&input, axis), InputGeometry { delay: None, action: None, delivery: None, immediate: false });
        // An old action may still have an in-window arrival, so retain its visible delay tail.
        input.arrived_ms = Some(11_000);
        let geometry = input_geometry(&input, axis);
        assert!(geometry.action.is_none());
        assert_eq!(geometry.delay, Some((axis.x(axis.start as u64), axis.x(11_000))));
        assert_eq!(geometry.delivery, Some(axis.x(11_000)));
        input.original_ms = 22_000;
        input.delivery_ms = 23_000;
        input.arrived_ms = None;
        assert_eq!(input_geometry(&input, axis), InputGeometry { delay: None, action: None, delivery: None, immediate: false });
    }
    #[test]
    fn every_service_has_two_separated_strips_with_contime_above_arrival() {
        for count in 1..=crate::uptime_report::MAX_SERVICES {
            for index in 0..count {
                let row = row(index, count);
                assert!(row.reconciled - row.height / 2.0 > row.arrival + row.height / 2.0);
                assert!(row.arrival - row.height / 2.0 > -47.0);
                assert!(row.height >= 0.5);
                if index > 0 {
                    let previous = super::row(index - 1, count);
                    assert!(previous.arrival - previous.height / 2.0 > row.reconciled + row.height / 2.0);
                }
            }
        }
    }
    #[test]
    fn zero_delay_geometry_has_no_invented_delay_span() {
        let axis = Axis { start: 0, end: 1000, left: 0.0, width: 100.0 };
        let input =
            Input { id: 1, service: 1, status: Status::Healthy, original_ms: 500, delivery_ms: 500, arrived_ms: Some(500), late: false };
        let geometry = input_geometry(&input, axis);
        assert!(geometry.immediate);
        assert!(geometry.delay.is_none());
        assert_eq!(geometry.action, geometry.delivery);
        let delayed = Input { delivery_ms: 800, arrived_ms: Some(800), ..input };
        let geometry = input_geometry(&delayed, axis);
        assert!(!geometry.immediate);
        assert_eq!(geometry.delay, Some((50.0, 80.0)));
    }
    #[test]
    fn shared_layout_keeps_delay_ticks_and_departing_symbols_in_the_same_tracks() {
        use crate::uptime_report::ServiceReport;
        let report = Report {
            clock_ms: 500,
            services: vec![ServiceReport {
                id: 1,
                name: "Service 1".into(),
                reconciled: Default::default(),
                arrival_order: Default::default(),
            }],
            inputs: vec![
                Input { id: 1, service: 1, status: Status::Down, original_ms: 100, delivery_ms: 600, arrived_ms: Some(500), late: true },
                Input { id: 2, service: 1, status: Status::Healthy, original_ms: 200, delivery_ms: 700, arrived_ms: None, late: false },
            ],
            ..Default::default()
        };
        let layout = NetworkLayout::new(&report);
        assert_ne!(layout.positions[0].y, layout.positions[1].y);
        for (i, input) in report.inputs.iter().enumerate() {
            assert_eq!(Some(layout.positions[i].x), input_geometry(input, layout.axis).delivery);
            assert!((layout.positions[i].y - layout.row_y(0)).abs() <= layout.spacing * 0.35 + 0.0001);
        }
    }
    #[test]
    fn all_supported_service_identities_are_distinct() {
        for id in 1..=crate::uptime_report::MAX_SERVICES {
            for earlier in 1..id {
                assert_ne!(identity(id, 1.0), identity(earlier, 1.0));
            }
        }
    }
    #[test]
    fn five_default_service_colors_are_well_separated() {
        for id in 1..=5 {
            let a = identity(id, 1.0).to_srgba();
            for earlier in 1..id {
                let b = identity(earlier, 1.0).to_srgba();
                let distance = (a.red - b.red).powi(2) + (a.green - b.green).powi(2) + (a.blue - b.blue).powi(2);
                assert!(distance > 0.28_f32.powi(2), "Services {earlier} and {id} need more distinct colors");
            }
        }
    }
    #[test]
    fn default_service_colors_are_readable_against_the_visualizer_background() {
        let background = Color::srgb_u8(21, 37, 53).to_srgba();
        let luminance = |color: bevy::color::Srgba| {
            let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * linear(color.red) + 0.7152 * linear(color.green) + 0.0722 * linear(color.blue)
        };
        for id in 1..=5 {
            let color = identity(id, 1.0).to_srgba();
            let contrast = (luminance(color) + 0.05) / (luminance(background) + 0.05);
            assert!(contrast >= 4.5, "Service {id} label contrast is {contrast}");
            for alpha in [PENDING_OPACITY, 0.85] {
                let line = bevy::color::Srgba::new(
                    color.red * alpha + background.red * (1.0 - alpha),
                    color.green * alpha + background.green * (1.0 - alpha),
                    color.blue * alpha + background.blue * (1.0 - alpha),
                    1.0,
                );
                assert!((luminance(line) + 0.05) / (luminance(background) + 0.05) >= 3.0);
            }
        }
    }
    #[test]
    fn identities_remain_separate_from_health_status_colors() {
        for id in 1..=crate::uptime_report::MAX_SERVICES {
            let color = identity(id, 1.0).to_srgba();
            let hsl = bevy::color::Hsla::from(identity(id, 1.0));
            let cool = (180.0..330.0).contains(&hsl.hue);
            let cocoa = (20.0..35.0).contains(&hsl.hue) && hsl.saturation < 0.55;
            let neutral = hsl.saturation < 0.20;
            assert!(cool || cocoa || neutral);
            assert_eq!(color.alpha, 1.0);
            for status in [Status::Healthy, Status::Degraded, Status::Down] {
                assert_ne!(identity(id, 1.0), health(status));
            }
        }
    }
    #[test]
    fn health_colors_are_distinct_from_each_other() {
        assert_ne!(health(Status::Healthy), health(Status::Degraded));
        assert_ne!(health(Status::Degraded), health(Status::Down));
        assert_ne!(health(Status::Down), health(Status::Healthy));
    }
}
