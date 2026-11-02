//! Shared navigation above the visualization viewport.
use crate::{
    bridge::Bridge,
    catalog::{Navigation, Visualization},
};
use bevy::prelude::*;
pub(crate) fn height(width: f32, active: Visualization) -> f32 {
    match (width < 640.0, active) {
        (true, Visualization::Uptime | Visualization::Accounts | Visualization::Robotics) => 320.0,
        (false, Visualization::Uptime | Visualization::Accounts | Visualization::Robotics) => 240.0,
        (true, Visualization::Movement) => 360.0,
        (false, Visualization::Movement) => 260.0,
    }
}
#[derive(Component)]
pub(crate) struct Bar;
#[derive(Component)]
pub(crate) struct Action(Navigation);
#[derive(Component)]
pub(crate) enum Copy {
    Title,
    Description,
    Status,
}

pub(crate) fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::from_bytes(include_bytes!("../assets/fonts/Inter-Regular.ttf").to_vec()));
    let text_font = |size: f32| TextFont { font: font.clone().into(), font_size: size.into(), ..default() };
    commands
        .spawn((
            Bar,
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                width: percent(100),
                height: px(240),
                padding: UiRect::axes(px(20), px(12)),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(17, 29, 41)),
        ))
        .with_children(|bar| {
            bar.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(12),
                align_items: AlignItems::Center,
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .with_children(|row| {
                for (action, label) in [(Navigation::Reset, "Reset"), (Navigation::Menu, "Back to visualizations")] {
                    row.spawn((
                        Button,
                        Action(action),
                        Node { padding: UiRect::axes(px(12), px(7)), ..default() },
                        BackgroundColor(Color::srgb(0.15, 0.28, 0.34)),
                    ))
                    .with_children(|button| {
                        button.spawn((Text::new(label), text_font(14.0), TextColor(Color::srgb(0.88, 0.94, 0.96))));
                    });
                }
                row.spawn((Copy::Status, Text::new(""), text_font(13.0), TextColor(Color::srgb(0.88, 0.78, 0.54))));
            });
            bar.spawn((Copy::Title, Text::new(""), text_font(22.0), TextColor(Color::srgb(0.91, 0.95, 0.97))));
            bar.spawn((Copy::Description, Text::new(""), text_font(14.0), TextColor(Color::srgb(0.69, 0.79, 0.83))));
        });
}

pub(crate) fn update(
    bridge: Res<Bridge>,
    windows: Query<&Window>,
    mut nodes: Query<(&mut Node, Option<&Bar>, Option<&Action>)>,
    mut copy: Query<(&Copy, &mut Text)>,
    mut buttons: Query<(&Action, Ref<Interaction>, &mut BackgroundColor)>,
) {
    let state = bridge.snapshot();
    let active = state.active();
    let width = windows.iter().next().map_or(1280.0, |window| window.width());
    for (mut node, bar, action) in &mut nodes {
        if bar.is_some() {
            if let Some(active) = active {
                node.height = px(height(width, active));
            }
            node.display = if active.is_some() && !state.choosing { Display::Flex } else { Display::None };
        }
        if let Some(Action(Navigation::Menu)) = action {
            node.display = if state.can_choose { Display::Flex } else { Display::None };
        }
    }
    if let Some(active) = active {
        let description = crate::explanation::description(active, &state);
        for (kind, mut text) in &mut copy {
            let value = match kind {
                Copy::Title => active.label(),
                Copy::Description => &description,
                Copy::Status if state.transitioning => "Finishing current example…",
                Copy::Status => "",
            };
            if text.0 != value {
                text.0 = value.to_owned();
            }
        }
    }
    for (action, interaction, mut color) in &mut buttons {
        let enabled = !state.choosing && active.is_some() && !state.transitioning && (action.0 != Navigation::Menu || state.can_choose);
        color.0 = if !enabled {
            Color::srgb(0.12, 0.18, 0.22)
        } else if *interaction == Interaction::Hovered {
            Color::srgb(0.22, 0.40, 0.46)
        } else {
            Color::srgb(0.15, 0.28, 0.34)
        };
        if enabled && interaction.is_changed() && *interaction == Interaction::Pressed {
            bridge.request_navigation(action.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finished_view_retains_navigation_and_held_press_does_not_restart_again() {
        let bridge = Bridge::default();
        bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
        bridge.finish();
        let mut app = App::new();
        app.insert_resource(bridge.clone()).init_resource::<Assets<Font>>().add_systems(Startup, setup).add_systems(Update, update);
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<&Node, With<Bar>>().single(world).unwrap().display, Display::Flex);
        for (action, mut interaction) in world.query::<(&Action, &mut Interaction)>().iter_mut(world) {
            if action.0 == Navigation::Reset {
                *interaction = Interaction::Pressed;
            }
        }
        app.update();
        assert_eq!(bridge.take_navigation(), Some(Navigation::Reset));
        bridge.restart_view();
        app.update();
        assert_eq!(bridge.take_navigation(), None);
    }
}
