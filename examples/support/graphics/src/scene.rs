use crate::{animation::Animation, bridge::Bridge};
use bevy::{camera::ScalingMode, math::Isometry2d, prelude::*};

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsSet {
    Collect,
    Draw,
}
#[derive(Default, Resource)]
struct Display {
    animation: Animation,
    transfers: crate::transfer::Transfers,
    entries: crate::entrance::Entries,
    tracks: crate::tracks::Tracks,
    generation: u64,
}

pub fn app(bridge: Bridge, canvas: Option<String>) -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "ConTime / Client movement".into(),
            canvas,
            resolution: (1000, 1000).into(),
            fit_canvas_to_parent: true,
            prevent_default_event_handling: false,
            ..default()
        }),
        close_when_requested: false,
        ..default()
    }))
    .insert_resource(ClearColor(Color::srgb_u8(21, 37, 53)))
    .insert_resource(bridge)
    .init_resource::<Display>()
    .configure_sets(Update, (GraphicsSet::Collect, GraphicsSet::Draw).chain())
    .add_systems(Startup, setup)
    .add_systems(Update, draw.in_set(GraphicsSet::Draw));
    app
}
fn setup(mut commands: Commands, mut config: ResMut<GizmoConfigStore>) {
    commands.spawn((
        Camera2d,
        Transform::from_xyz(0.0, 20.0, 0.0),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: 106.0, min_height: 116.0 },
            ..OrthographicProjection::default_2d()
        }),
    ));
    config.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 2.0;
}
fn draw(bridge: Res<Bridge>, mut display: ResMut<Display>, time: Res<Time>, mut gizmos: Gizmos) {
    let state = bridge.snapshot();
    if display.generation != state.generation {
        display.animation.reset();
        display.transfers.reset();
        display.entries.reset();
        display.tracks.reset();
        display.generation = state.generation;
    }
    let Some(report) = state.report else {
        return;
    };
    let elapsed = if state.running { time.delta_secs_f64() * 1000.0 } else { 0.0 };
    display.animation.update(&report, elapsed, state.reduced_motion || state.finished);
    display.tracks.update(&report);
    display.entries.update(&report, elapsed, state.reduced_motion || state.finished);
    // Delivery effects follow the completed timeline entrance and the drawn track.
    let Display { transfers, tracks, entries, .. } = &mut *display;
    transfers.update(&report, tracks, entries, elapsed, state.reduced_motion || state.finished);
    let [hx, hy] = report.arena_half_extent.map(|v| v as f32);
    let color = |r, g, b, alpha| Color::srgba(r, g, b, alpha);
    let muted = color(0.50, 0.65, 0.72, 1.0);
    let anchor = Vec2::new(-0.5, 0.0);
    let grid = color(0.13, 0.21, 0.28, 1.0);
    for x in ((-hx / 10.0).ceil() as i32)..=(hx / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(x as f32 * 10.0, -hy), Vec2::new(x as f32 * 10.0, hy), grid);
    }
    for y in ((-hy / 10.0).ceil() as i32)..=(hy / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(-hx, y as f32 * 10.0), Vec2::new(hx, y as f32 * 10.0), grid);
    }
    gizmos.rect_2d(Isometry2d::IDENTITY, Vec2::new(hx * 2.0, hy * 2.0), color(0.42, 0.55, 0.62, 1.0));
    crate::paths::draw(&report, state.reduced_motion, &mut gizmos);
    crate::network::draw(&report, &display.entries, &display.tracks, state.reduced_motion, &mut gizmos);
    for (y, label) in [
        (-33.5, "Client colors: queried paths / Gray: revised old paths / Light arrows or squares: delivered inputs"),
        (-36.0, "Ring: you / Arrow: direction / Dot: Stop / Square: past Stop / WASD or arrows, Space stops"),
    ] {
        gizmos.text_2d(Isometry2d::from_translation(Vec2::new(-hx, y)), label, 0.9, anchor, muted);
    }
    for (id, marker) in &display.animation.clients {
        let p = Vec2::from_array(marker.position.map(|v| v as f32));
        let c = crate::palette::input(*id, marker.opacity as f32);
        let direction = Vec2::from_array(marker.direction.map(|v| v as f32));
        if direction == Vec2::ZERO {
            gizmos.circle_2d(p, 0.40, c).resolution(16);
            gizmos.circle_2d(p, 0.22, c).resolution(8);
        } else {
            let tip = p + direction * 0.85;
            let shoulder = p - direction * 0.07;
            gizmos.line_2d(p - direction * 0.65, tip, c);
            gizmos.line_2d(shoulder + direction.perp() * 0.45, tip, c);
            gizmos.line_2d(shoulder - direction.perp() * 0.45, tip, c);
        }
        if *id == 1 {
            gizmos.circle_2d(p, 1.35, color(0.78, 0.87, 0.90, marker.opacity as f32 * 0.7)).resolution(24);
        }
    }
    display.transfers.draw(&mut gizmos);
}

#[cfg(test)]
use crate::paths::ordered_trails;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_queried_paths_cover_revised_paths_at_the_same_positions() {
        let mut report = crate::animation::tests::scene(0.0);
        report.trails = vec![
            crate::report::Point {
                previous: Some([1.0, 3.0]),
                client: 1,
                time_ns: 1,
                position: [2.0, 3.0],
                current: true,
                corrected: false,
                opacity: 1.0,
            },
            crate::report::Point {
                previous: Some([1.0, 3.0]),
                client: 1,
                time_ns: 1,
                position: [2.0, 3.0],
                current: false,
                corrected: true,
                opacity: 0.5,
            },
        ];
        let points: Vec<_> = ordered_trails(&report).collect();
        assert!(!points[0].current && points[1].current);
        assert_eq!(points[0].position, points[1].position);
        assert_eq!(points[1].opacity, 1.0);
        assert_eq!(points[0].previous, points[1].previous);
    }
}
