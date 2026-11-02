//! Shared, screen-sized chooser for the desktop and browser presentation.
use crate::{bridge::Bridge, catalog::Visualization};
use bevy::prelude::*;

#[derive(Default, Resource)]
pub(crate) struct Focus(usize);
#[derive(Component)]
pub(crate) struct Chooser;
#[derive(Component)]
pub(crate) struct Choice(Visualization);
#[derive(Component)]
pub(crate) struct Loading;

pub(crate) fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::from_bytes(include_bytes!("../assets/fonts/Inter-Regular.ttf").to_vec()));
    let text_font = |size: f32| TextFont { font: font.clone().into(), font_size: size.into(), ..default() };
    commands
        .spawn((
            Chooser,
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(16)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn(Node {
                width: percent(100),
                max_width: px(640),
                flex_direction: FlexDirection::Column,
                row_gap: px(12),
                ..default()
            })
            .with_children(|panel| {
                panel.spawn((Text::new("ConTime"), text_font(18.0), TextColor(Color::srgb(0.58, 0.77, 0.82))));
                panel.spawn((Text::new("Choose a visualization"), text_font(30.0), TextColor(Color::srgb(0.91, 0.95, 0.97))));
                panel.spawn((
                    Text::new("Explore how late events change the past."),
                    text_font(16.0),
                    TextColor(Color::srgb(0.67, 0.76, 0.81)),
                ));
                for (index, visualization) in Visualization::ALL.into_iter().enumerate() {
                    panel
                        .spawn((
                            Button,
                            Choice(visualization),
                            Node {
                                width: percent(100),
                                padding: UiRect::all(px(16)),
                                flex_direction: FlexDirection::Column,
                                row_gap: px(7),
                                border: UiRect::all(px(2)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.10, 0.18, 0.24)),
                            BorderColor::all(Color::srgb(0.24, 0.36, 0.43)),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new(format!("{}  {}", index + 1, visualization.label())),
                                text_font(20.0),
                                TextColor(Color::srgb(0.91, 0.95, 0.97)),
                            ));
                            button.spawn((
                                Text::new(visualization.description()),
                                text_font(14.0),
                                TextColor(Color::srgb(0.69, 0.79, 0.83)),
                            ));
                        });
                }
                panel.spawn((
                    Text::new("Click to start · 1 / 2 / 3 / 4 · ↑ / ↓ then Enter"),
                    text_font(14.0),
                    TextColor(Color::srgb(0.60, 0.70, 0.76)),
                ));
            });
        });
    commands.spawn((
        Loading,
        Text::new("Starting visualization…"),
        text_font(24.0),
        TextColor(Color::srgb(0.82, 0.90, 0.94)),
        Node { position_type: PositionType::Absolute, left: px(32), top: px(164), display: Display::None, ..default() },
    ));
}

#[allow(clippy::type_complexity)] // Disjoint root and loading UI nodes.
pub(crate) fn update(
    bridge: Res<Bridge>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<Focus>,
    mut roots: Query<&mut Node, With<Chooser>>,
    mut loading: Query<(&mut Node, &mut Text), (With<Loading>, Without<Chooser>)>,
    mut buttons: Query<(&Choice, &Interaction, &mut BackgroundColor, &mut BorderColor)>,
) {
    let state = bridge.snapshot();
    for mut root in &mut roots {
        root.display = if state.choosing { Display::Flex } else { Display::None };
    }
    for (mut node, mut text) in &mut loading {
        node.display = if !state.choosing
            && state.selected.is_some()
            && state.report.is_none()
            && state.uptime.is_none()
            && state.accounts.is_none()
            && state.robotics.is_none()
        {
            Display::Flex
        } else {
            Display::None
        };
        if let Some(selected) = state.selected {
            text.0 = format!("Starting {}…", selected.label().to_lowercase());
        }
    }
    if !state.choosing {
        return;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        focus.0 = (focus.0 + Visualization::ALL.len() - 1) % Visualization::ALL.len();
    } else if keys.just_pressed(KeyCode::ArrowDown) {
        focus.0 = (focus.0 + 1) % Visualization::ALL.len();
    }
    let direct = if keys.just_pressed(KeyCode::Digit1) {
        Some(Visualization::Movement)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(Visualization::Uptime)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(Visualization::Accounts)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(Visualization::Robotics)
    } else if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        Some(Visualization::ALL[focus.0])
    } else {
        None
    };
    if let Some(selected) = direct {
        bridge.select(selected);
    }
    for (choice, interaction, mut background, mut border) in &mut buttons {
        if *interaction == Interaction::Pressed {
            bridge.select(choice.0);
        }
        let highlighted = *interaction != Interaction::None || choice.0 == Visualization::ALL[focus.0];
        background.0 = if highlighted { Color::srgb(0.14, 0.26, 0.32) } else { Color::srgb(0.10, 0.18, 0.24) };
        *border = BorderColor::all(if highlighted { Color::srgb(0.39, 0.80, 0.85) } else { Color::srgb(0.24, 0.36, 0.43) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let bridge = Bridge::default();
        bridge.show_chooser();
        let mut app = App::new();
        app.insert_resource(bridge)
            .init_resource::<Assets<Font>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Focus>()
            .add_systems(Startup, setup)
            .add_systems(Update, update);
        app.update();
        app
    }
    #[test]
    fn keyboard_choice_hides_menu_and_shows_loading_without_a_report() {
        let mut app = app();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::ArrowDown);
        app.update();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().clear();
        app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::Enter);
        app.update();
        let bridge = app.world().resource::<Bridge>().clone();
        assert_eq!(bridge.take_selection(), Some(Visualization::Uptime));
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query_filtered::<&Node, With<Chooser>>().single(world).unwrap().display, Display::None);
        assert_eq!(world.query_filtered::<&Node, With<Loading>>().single(world).unwrap().display, Display::Flex);
    }
    #[test]
    fn pointer_press_selects_exactly_once() {
        let mut app = app();
        let world = app.world_mut();
        for (choice, mut interaction) in world.query::<(&Choice, &mut Interaction)>().iter_mut(world) {
            if choice.0 == Visualization::Movement {
                *interaction = Interaction::Pressed;
            }
        }
        app.update();
        let bridge = app.world().resource::<Bridge>();
        assert_eq!(bridge.take_selection(), Some(Visualization::Movement));
        assert_eq!(bridge.take_selection(), None);
        assert!(bridge.snapshot().report.is_none());
    }
}
