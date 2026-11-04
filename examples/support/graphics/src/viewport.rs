//! Fit the diagrams around the description and controls without distortion.
use crate::bridge::Bridge;
use bevy::{camera::ScalingMode, prelude::*, window::PrimaryWindow};
pub(crate) fn dashboard_compact(size: Vec2) -> bool {
    let usable_height = (size.y - crate::navigation::height(size.x) - crate::navigation::footer_height(size.x) - 32.0).max(1.0);
    let projected_width = size.x.min(214.0 * usable_height / 186.0);
    projected_width < 700.0
}
fn layout(size: Vec2, header: f32, footer: f32, dashboard: bool, bottom_description: bool) -> Option<(f32, f32, f32)> {
    if !size.is_finite() || size.x <= 0.0 || size.y <= 0.0 {
        return None;
    }
    let top = if dashboard && !bottom_description { header + 16.0 } else { 8.0 };
    let bottom = footer + 16.0 + if dashboard && !bottom_description { 0.0 } else { header };
    let compact = dashboard && dashboard_compact(size);
    let world_height = if compact {
        246.0
    } else if dashboard {
        186.0
    } else {
        110.0
    };
    let scale = (214.0 / size.x).max(world_height / (size.y - top - bottom).max(1.0));
    let world_top = if compact {
        120.0
    } else if dashboard {
        96.0
    } else {
        77.5
    };
    Some((size.x * scale, size.y * scale, world_top - size.y * scale * 0.5 + top * scale))
}
pub(crate) fn dashboard_bottom(size: Vec2) -> f32 {
    let header = crate::navigation::height(size.x);
    let footer = crate::navigation::footer_height(size.x);
    let Some((_, height, center)) = layout(size, header, footer, true, true) else { return -90.0 };
    let bottom_px = footer + header + 16.0;
    center - (0.5 - bottom_px / size.y) * height
}
pub(crate) fn dashboard_horizontal_scale(size: Vec2) -> f32 {
    layout(size, crate::navigation::height(size.x), crate::navigation::footer_height(size.x), true, true)
        .map_or(1.0, |(width, _, _)| (width / 214.0).max(1.0))
}
pub(crate) fn update(
    windows: Query<&Window, With<PrimaryWindow>>,
    bridge: Res<Bridge>,
    mut cameras: Query<(&mut Projection, &mut Transform), With<Camera2d>>,
) {
    let Ok(window) = windows.single() else { return };
    let size = Vec2::new(window.width(), window.height());
    let state = bridge.snapshot();
    let dashboard = state.dashboard.is_some();
    let bottom_description = state.dashboard.as_ref().is_none_or(|data| data.hash);
    let Some((width, height, center_y)) =
        layout(size, crate::navigation::height(size.x), crate::navigation::footer_height(size.x), dashboard, bottom_description)
    else {
        return;
    };
    for (mut projection, mut transform) in &mut cameras {
        if let Projection::Orthographic(ref mut o) = *projection {
            o.scaling_mode = ScalingMode::Fixed { width, height };
            transform.translation.y = center_y;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_fits_above_description_without_wasting_horizontal_space() {
        for size in [Vec2::new(1200.0, 1000.0), Vec2::new(390.0, 844.0)] {
            let header = crate::navigation::height(size.x);
            let (width, height, center) = layout(size, header, crate::navigation::footer_height(size.x), false, true).unwrap();
            let y = |world: f32| size.y * (0.5 - (world - center) / height);
            assert!(width <= 220.0, "diagram should fill width: {size:?} {width}");
            assert!(y(77.5) >= 7.99);
            assert!(y(-30.0) < size.y - crate::navigation::footer_height(size.x) - header);
        }
    }
    #[test]
    fn dashboard_layout_accounts_for_available_height() {
        assert!(dashboard_compact(Vec2::new(1280.0, 720.0)));
        assert!(dashboard_compact(Vec2::new(390.0, 844.0)));
        assert!(!dashboard_compact(Vec2::new(1200.0, 900.0)));
    }
    #[test]
    fn dashboard_fills_the_available_width_even_when_height_limited() {
        for size in [Vec2::new(1280.0, 720.0), Vec2::new(1200.0, 900.0), Vec2::new(390.0, 844.0)] {
            let (width, _, _) =
                layout(size, crate::navigation::height(size.x), crate::navigation::footer_height(size.x), true, true).unwrap();
            let scale = dashboard_horizontal_scale(size);
            assert!((214.0 * scale / width - 1.0).abs() < 0.001);
        }
    }
    #[test]
    fn rejects_invalid_dimensions() {
        assert!(layout(Vec2::ZERO, 85.0, 110.0, false, true).is_none());
    }
}
