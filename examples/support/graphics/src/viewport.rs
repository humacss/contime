//! Keep the world display below screen-sized navigation without moving UI labels.
use crate::{bridge::Bridge, catalog::Visualization};
use bevy::{camera::ScalingMode, prelude::*, window::PrimaryWindow};

struct View {
    width: f32,
    height: f32,
    center_y: f32,
}
fn layout(size: Vec2, navigation: Option<Visualization>) -> Option<View> {
    if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let top = navigation.map_or(0.0, |active| crate::navigation::height(size.x, active) + 16.0);
    let scale = (214.0 / size.x).max(156.0 / (size.y - top).max(1.0));
    // Start the diagrams just below the explanation, including on tall windows.
    let center_y = if navigation.is_some() { 98.0 - size.y * scale * 0.5 + top * scale } else { 20.0 };
    Some(View { width: size.x * scale, height: size.y * scale, center_y })
}
pub(crate) fn update(
    windows: Query<&Window, With<PrimaryWindow>>,
    bridge: Res<Bridge>,
    mut cameras: Query<(&mut Projection, &mut Transform), With<Camera2d>>,
) {
    let Ok(window) = windows.single() else { return };
    let state = bridge.snapshot();
    let Some(view) = layout(Vec2::new(window.width(), window.height()), state.active().filter(|_| !state.choosing)) else { return };
    for (mut projection, mut transform) in &mut cameras {
        if let Projection::Orthographic(ref mut orthographic) = *projection {
            orthographic.scaling_mode = ScalingMode::Fixed { width: view.width, height: view.height };
            transform.translation.y = view.center_y;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_displays_fit_below_navigation_on_desktop_and_mobile() {
        for size in [Vec2::new(1440.0, 900.0), Vec2::new(1014.0, 1342.0), Vec2::new(390.0, 844.0)] {
            let view = layout(size, Some(Visualization::Uptime)).unwrap();
            let screen_y = |world_y| size.y * (0.5 - (world_y - view.center_y) / view.height);
            assert!(
                screen_y(98.0) >= crate::navigation::height(size.x, Visualization::Uptime),
                "world titles overlap the navigation at {size}"
            );
            assert!(screen_y(-58.0) <= size.y + 0.01);
            assert!(view.width >= 214.0);
        }
    }
    #[test]
    fn chooser_uses_the_full_window_and_invalid_sizes_are_ignored() {
        let view = layout(Vec2::new(1440.0, 900.0), None).unwrap();
        assert_eq!(view.center_y, 20.0);
        assert!((view.height - 156.0).abs() < 0.01);
        for size in [Vec2::ZERO, Vec2::new(f32::NAN, 900.0), Vec2::new(1000.0, -1.0)] {
            assert!(layout(size, Some(Visualization::Uptime)).is_none());
        }
    }
}
