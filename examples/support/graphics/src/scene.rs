use crate::{
    animation::Animation,
    bridge::Bridge,
    hud::{COLUMNS, Hud, LABELS},
};
use bevy::{camera::ScalingMode, math::Isometry2d, prelude::*};

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsSet {
    Collect,
    Draw,
}
#[derive(Default, Resource)]
struct Display {
    animation: Animation,
    generation: u64,
}

pub fn app(bridge: Bridge, canvas: Option<String>) -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "ConTime / Client movement".into(),
            canvas,
            resolution: (1000, 660).into(),
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
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: 106.0, min_height: 76.0 },
            ..OrthographicProjection::default_2d()
        }),
    ));
    config.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 2.0;
}
fn draw(bridge: Res<Bridge>, mut display: ResMut<Display>, time: Res<Time>, mut gizmos: Gizmos) {
    let state = bridge.snapshot();
    if display.generation != state.generation {
        display.animation.reset();
        display.generation = state.generation;
    }
    let Some(report) = state.report else {
        return;
    };
    let elapsed = if state.running { time.delta_secs_f64() * 1000.0 } else { 0.0 };
    display.animation.update(&report, elapsed, state.reduced_motion || state.finished);
    let [hx, hy] = report.arena_half_extent.map(|v| v as f32);
    let color = |r, g, b, alpha| Color::srgba(r, g, b, alpha);
    let hud = Hud::new(
        &report,
        if state.finished {
            "Validated"
        } else if state.running {
            "Live"
        } else {
            "Paused"
        },
    );
    let text = color(0.73, 0.83, 0.87, 1.0);
    let muted = color(0.50, 0.65, 0.72, 1.0);
    let anchor = Vec2::new(-0.5, 0.0);
    gizmos.text_2d(Isometry2d::from_translation(Vec2::new(-hx, 36.5)), &hud.settings, 1.25, anchor, text);
    for ((x, label), value) in COLUMNS.into_iter().zip(LABELS).zip(&hud.values) {
        gizmos.text_2d(Isometry2d::from_translation(Vec2::new(x, 34.0)), label, 1.15, anchor, muted);
        gizmos.text_2d(Isometry2d::from_translation(Vec2::new(x, 31.5)), value, 1.8, anchor, text);
    }
    gizmos.text_2d(Isometry2d::from_translation(Vec2::new(-hx, -32.5)), &hud.footer, 1.1, anchor, muted);
    let grid = color(0.13, 0.21, 0.28, 1.0);
    for x in ((-hx / 10.0).ceil() as i32)..=(hx / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(x as f32 * 10.0, -hy), Vec2::new(x as f32 * 10.0, hy), grid);
    }
    for y in ((-hy / 10.0).ceil() as i32)..=(hy / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(-hx, y as f32 * 10.0), Vec2::new(hx, y as f32 * 10.0), grid);
    }
    gizmos.rect_2d(Isometry2d::IDENTITY, Vec2::new(hx * 2.0, hy * 2.0), color(0.42, 0.55, 0.62, 1.0));
    // Current points are opaque and drawn last at the very same geometry.
    for trail in ordered_trails(&report) {
        let current = trail.current;
        let p = Vec2::from_array(trail.position.map(|v| v as f32));
        let c = if current { color(0.29, 0.84, 0.87, 1.0) } else { color(0.91, 0.67, 0.23, trail.opacity as f32 * 0.5) };
        gizmos.circle_2d(p, 0.10, c).resolution(6);
    }
    for (id, marker) in &display.animation.clients {
        let p = Vec2::from_array(marker.position.map(|v| v as f32));
        let c = if *id == 1 { color(1.0, 0.85, 0.41, marker.opacity as f32) } else { color(0.29, 0.84, 0.87, marker.opacity as f32) };
        gizmos.circle_2d(p, 0.40, c).resolution(16);
        gizmos.circle_2d(p, 0.16, c).resolution(8);
        gizmos.circle_2d(p, 0.72, color(0.91, 0.67, 0.23, (marker.opacity * marker.correction * 0.7) as f32)).resolution(20);
        let label = if *id == 1 { '@' } else { char::from(b'a' + ((id - 1) % 26) as u8) };
        gizmos.text_2d(Isometry2d::from_translation(p + Vec2::Y * 1.1), &label.to_string(), 1.3, Vec2::ZERO, c);
    }
}

fn ordered_trails(report: &crate::report::Report) -> impl Iterator<Item = &crate::report::Point> {
    report.trails.iter().filter(|point| !point.current).chain(report.trails.iter().filter(|point| point.current))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_queried_paths_cover_revised_paths_at_the_same_positions() {
        let mut report = crate::animation::tests::scene(0.0);
        report.trails = vec![
            crate::report::Point { position: [2.0, 3.0], current: true, corrected: false, opacity: 1.0 },
            crate::report::Point { position: [2.0, 3.0], current: false, corrected: true, opacity: 0.5 },
        ];
        let points: Vec<_> = ordered_trails(&report).collect();
        assert!(!points[0].current && points[1].current);
        assert_eq!(points[0].position, points[1].position);
        assert_eq!(points[1].opacity, 1.0);
    }
}
