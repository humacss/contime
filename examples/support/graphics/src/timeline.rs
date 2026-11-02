//! Domain-neutral timeline placement and delivery marks.
use bevy::prelude::*;
pub(crate) fn time_x(time: u64, origin: u64, past: f64, future: f64, left: f32, width: f32) -> f32 {
    let relative = (i128::from(time) - i128::from(origin)) as f64;
    left + width * ((relative + past) / (past + future).max(1.0)).clamp(0.0, 1.0) as f32
}
pub(crate) fn delivery_tick(gizmos: &mut Gizmos, point: Vec2, size: f32, reordered: bool, color: Color) {
    for offset in if reordered { &[-0.20, 0.20][..] } else { &[0.0][..] } {
        let tick = point + Vec2::X * offset;
        gizmos.line_2d(tick - Vec2::Y * size, tick + Vec2::Y * size, color);
    }
}
