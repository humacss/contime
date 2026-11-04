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
    clock: crate::animation::FrameClock,
    playback: crate::playback::Playback,
    unreconciled_animation: Animation,
    transfers: crate::transfer::Transfers,
    unreconciled_transfers: crate::transfer::Transfers,
    entries: crate::entrance::Entries,
    tracks: crate::tracks::Tracks,
    generation: u64,
    dashboard_view: crate::dashboard_view::View,
}

pub fn app(bridge: Bridge, canvas: Option<String>) -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "ConTime / Client movement".into(),
            canvas,
            resolution: (1440, 900).into(),
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
    .init_resource::<crate::labels::Labels>()
    .configure_sets(Update, (GraphicsSet::Collect, GraphicsSet::Draw).chain())
    .add_systems(Startup, (setup, crate::labels::setup, crate::navigation::setup))
    .add_systems(Update, crate::navigation::update.before(GraphicsSet::Collect))
    .add_systems(Update, crate::viewport::update.in_set(GraphicsSet::Collect))
    .add_systems(
        PostUpdate,
        crate::labels::draw
            .after(bevy::camera::CameraUpdateSystems)
            .before(bevy::ui::UiSystems::Prepare)
            .before(bevy::text::detect_text_needs_rerender),
    )
    .add_systems(Update, draw.in_set(GraphicsSet::Draw));
    app
}
fn setup(mut commands: Commands, mut config: ResMut<GizmoConfigStore>) {
    commands.spawn((
        Camera2d,
        Transform::from_xyz(0.0, 20.0, 0.0),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin { min_width: 214.0, min_height: 156.0 },
            ..OrthographicProjection::default_2d()
        }),
    ));
    config.config_mut::<DefaultGizmoConfigGroup>().0.line.width = 2.0;
}
#[allow(clippy::too_many_arguments)] // Rendering gathers the shared scene inputs from Bevy.
fn draw(
    bridge: Res<Bridge>,
    mut display: ResMut<Display>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut labels: ResMut<crate::labels::Labels>,
    mut gizmos: Gizmos,
) {
    labels.clear();
    let state = bridge.snapshot();
    if display.generation != state.generation {
        display.animation.reset();
        display.clock = crate::animation::FrameClock::default();
        display.playback.reset();
        display.unreconciled_animation.reset();
        display.transfers.reset();
        display.unreconciled_transfers.reset();
        display.entries.reset();
        display.tracks.reset();
        display.generation = state.generation;
        display.dashboard_view.reset();
    }
    if let Some(data) = &state.dashboard {
        let window = windows.iter().next();
        let cursor = window.and_then(|w| w.cursor_position()).and_then(|position| {
            cameras.iter().next().and_then(|(camera, transform)| camera.viewport_to_world_2d(transform, position).ok())
        });
        display.dashboard_view.draw(
            data,
            &keys,
            &mut labels,
            &mut gizmos,
            window.map_or(Vec2::new(1280.0, 900.0), |w| Vec2::new(w.width(), w.height())),
            crate::dashboard_view::Pointer { position: cursor, clicked: mouse.just_pressed(MouseButton::Left) },
        );
        return;
    }
    let Some(report) = state.report else {
        return;
    };
    let elapsed = display.clock.elapsed(state.running, state.resume_epoch, time.delta_secs_f64() * 1000.0);
    let immediate = state.reduced_motion || state.finished;
    let (visible, handoff) = display.playback.update(std::sync::Arc::clone(&report), elapsed, immediate);
    let (left, right) = crate::comparison::panels(&visible);
    let (left_handoff, right_handoff) = crate::comparison::panels(&handoff);
    display.animation.update(&left, elapsed, immediate);
    display.unreconciled_animation.update(&right, elapsed, immediate);
    display.tracks.update(&report);
    display.entries.update(&report, elapsed, immediate);
    display.entries.handoff(&handoff);
    let Display { transfers, unreconciled_transfers, tracks, entries, .. } = &mut *display;
    transfers.update(&left_handoff, tracks, entries, elapsed, immediate);
    unreconciled_transfers.update(&right_handoff, tracks, entries, elapsed, immediate);
    draw_arena(&left, -crate::comparison::OFFSET as f32, "With ConTime", &mut labels, &mut gizmos);
    draw_arena(&right, crate::comparison::OFFSET as f32, "Without ConTime", &mut labels, &mut gizmos);
    crate::paths::draw_intended(&left, &mut gizmos);
    crate::paths::draw_intended(&right, &mut gizmos);
    crate::paths::draw(&left, state.reduced_motion, &mut gizmos);
    crate::paths::draw(&right, state.reduced_motion, &mut gizmos);
    crate::network::draw(&report, &display.entries, &display.tracks, state.reduced_motion, &mut labels, &mut gizmos);
    if !state.running && !state.finished {
        labels.push(Vec2::new(55.0, 74.0), 40.0, "PAUSED", crate::labels::Style::Heading, Color::srgb(0.95, 0.85, 0.55));
    }
    draw_markers(&display.animation, &mut gizmos);
    draw_markers(&display.unreconciled_animation, &mut gizmos);
    display.transfers.draw(&mut gizmos);
    display.unreconciled_transfers.draw(&mut gizmos);
}

fn draw_arena(report: &crate::report::Report, offset: f32, label: &str, labels: &mut crate::labels::Labels, gizmos: &mut Gizmos) {
    let [hx, hy] = report.arena_half_extent.map(|v| v as f32);
    let grid = Color::srgb(0.13, 0.21, 0.28);
    for x in ((-hx / 10.0).ceil() as i32)..=(hx / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(offset + x as f32 * 10.0, -hy), Vec2::new(offset + x as f32 * 10.0, hy), grid);
    }
    for y in ((-hy / 10.0).ceil() as i32)..=(hy / 10.0).floor() as i32 {
        gizmos.line_2d(Vec2::new(offset - hx, y as f32 * 10.0), Vec2::new(offset + hx, y as f32 * 10.0), grid);
    }
    gizmos.rect_2d(Isometry2d::from_translation(Vec2::new(offset, 0.0)), Vec2::new(hx * 2.0, hy * 2.0), Color::srgb(0.42, 0.55, 0.62));
    let compact = if label == "Without ConTime" { "Without ConTime" } else { label };
    labels.responsive(
        Vec2::new(offset - hx, hy + 4.5),
        hx * 2.0,
        label,
        compact,
        crate::labels::Style::Heading,
        Color::srgb(0.78, 0.87, 0.90),
    );
}

fn draw_markers(animation: &Animation, gizmos: &mut Gizmos) {
    for (id, marker) in &animation.clients {
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
            gizmos.circle_2d(p, 1.35, Color::srgba(0.78, 0.87, 0.90, marker.opacity as f32 * 0.7)).resolution(24);
        }
    }
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
