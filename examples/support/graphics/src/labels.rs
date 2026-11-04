//! Screen-sized Bevy UI labels anchored to the shared arena geometry.
use bevy::prelude::*;
const MAX_LABELS: usize = 144;
#[derive(Clone, Copy)]
pub(crate) enum Style {
    Heading,
    Caption,
    Legend,
    Status,
}
impl Style {
    fn size(self, width: f32) -> f32 {
        match self {
            Self::Heading if width < 640.0 => 15.0,
            Self::Heading if width < 800.0 => 17.0,
            Self::Heading => 20.0,
            Self::Caption if width < 640.0 => 14.0,
            Self::Caption => 16.0,
            Self::Legend if width < 640.0 => 12.0,
            Self::Legend => 14.0,
            Self::Status => 12.0,
        }
    }
}
struct Label {
    position: Vec2,
    width: f32,
    text: String,
    compact: Option<String>,
    style: Style,
    color: Color,
}
#[derive(Default, Resource)]
pub(crate) struct Labels(Vec<Label>);
impl Labels {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn push(&mut self, position: Vec2, width: f32, text: &str, style: Style, color: Color) {
        if self.0.len() < MAX_LABELS {
            self.0.push(Label { position, width, text: text.into(), compact: None, style, color });
        }
    }
    pub fn responsive(&mut self, position: Vec2, width: f32, text: &str, compact: &str, style: Style, color: Color) {
        if self.0.len() >= MAX_LABELS {
            return;
        }
        self.push(position, width, text, style, color);
        if let Some(label) = self.0.last_mut() {
            label.compact = Some(compact.into());
        }
    }
}
#[derive(Component)]
pub(crate) struct Slot(usize);

pub(crate) fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::from_bytes(include_bytes!("../assets/fonts/Inter-Regular.ttf").to_vec()));
    // Reuse UI entities; never recreate text entities on each rendered frame.
    for index in 0..MAX_LABELS {
        commands.spawn((
            Slot(index),
            Text::new(""),
            TextFont { font: font.clone().into(), font_size: 16.0.into(), ..default() },
            TextColor(Color::WHITE),
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
        ));
    }
}
#[allow(clippy::type_complexity)] // One bounded pool of UI text entities.
pub(crate) fn draw(
    labels: Res<Labels>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    windows: Query<&Window>,
    mut slots: Query<(&Slot, &mut Text, &mut TextFont, &mut TextColor, &mut Node, &mut Visibility)>,
) {
    let Some((camera, transform)) = cameras.iter().next() else {
        return;
    };
    let width = windows.iter().next().map_or(1000.0, |window| window.width());
    for (slot, mut text, mut font, mut color, mut node, mut visibility) in &mut slots {
        let projected = labels.0.get(slot.0).and_then(|label| {
            let start = camera.world_to_viewport(transform, label.position.extend(0.0)).ok()?;
            let end = camera.world_to_viewport(transform, (label.position + Vec2::X * label.width).extend(0.0)).ok()?;
            Some((label, start, (end.x - start.x).max(1.0)))
        });
        let Some((label, position, available)) = projected else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        let size = label.style.size(width);
        let left = px(position.x.round());
        let top = px((position.y - size * 0.6).round());
        let available = px(available.floor());
        let compact = match label.style {
            Style::Heading => width < 900.0,
            Style::Caption => width < 640.0,
            _ => false,
        };
        let value = if compact { label.compact.as_ref().unwrap_or(&label.text) } else { &label.text };
        if text.0 != *value {
            text.0.clone_from(value);
        }
        if font.font_size != size.into() {
            font.font_size = size.into();
        }
        if color.0 != label.color {
            color.0 = label.color;
        }
        if node.left != left || node.top != top || node.width != available {
            node.left = left;
            node.top = top;
            node.width = available;
        }
        if *visibility != Visibility::Visible {
            *visibility = Visibility::Visible;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_labels_cannot_grow_the_entity_pool_without_bound() {
        let mut labels = Labels::default();
        for _ in 0..1000 {
            labels.push(Vec2::ZERO, 96.0, "test", Style::Caption, Color::WHITE);
        }
        assert_eq!(labels.0.len(), MAX_LABELS);
        labels.clear();
        assert!(labels.0.is_empty());
    }
}
