//! Shared explanation, legend and reset control.
use crate::bridge::Bridge;
use bevy::prelude::*;

pub(crate) fn height(width: f32) -> f32 {
    if width < 400.0 {
        170.0
    } else if width < 600.0 {
        145.0
    } else if width < 900.0 {
        110.0
    } else {
        85.0
    }
}
pub(crate) fn footer_height(width: f32) -> f32 {
    if width < 400.0 {
        210.0
    } else if width < 600.0 {
        180.0
    } else if width < 900.0 {
        140.0
    } else {
        110.0
    }
}
#[derive(Component)]
pub(crate) struct Bar;
#[derive(Component)]
pub(crate) struct Footer;
#[derive(Component)]
pub(crate) struct Reset;
#[derive(Component)]
pub(crate) struct Title;
#[derive(Component)]
pub(crate) struct Description;
#[derive(Component)]
pub(crate) struct Help;

pub(crate) fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::from_bytes(include_bytes!("../assets/fonts/Inter-Regular.ttf").to_vec()));
    let text_font = |size: f32| TextFont { font: font.clone().into(), font_size: size.into(), ..default() };
    commands
        .spawn((
            Bar,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(110),
                left: px(0),
                width: percent(100),
                height: px(85),
                padding: UiRect::axes(px(20), px(12)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgb_u8(21, 37, 53)),
        ))
        .with_children(|bar| {
            bar.spawn((
                Title,
                Text::new(""),
                text_font(20.0),
                TextColor(Color::srgb(0.91, 0.95, 0.97)),
                Node { display: Display::None, ..default() },
            ));
            bar.spawn((Description, Text::new(""), text_font(15.0), TextColor(Color::srgb(0.82, 0.87, 0.89))));
        });
    commands
        .spawn((
            Footer,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(0),
                left: px(0),
                width: percent(100),
                height: px(110),
                padding: UiRect::axes(px(20), px(10)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(24),
                ..default()
            },
            BackgroundColor(Color::srgb_u8(17, 29, 41)),
        ))
        .with_children(|footer| {
            footer.spawn((
                Help,
                Text::new(""),
                text_font(14.0),
                TextColor(Color::srgb(0.75, 0.82, 0.85)),
                Node { flex_grow: 1.0, flex_shrink: 1.0, min_width: px(0), ..default() },
            ));
            footer
                .spawn((
                    Reset,
                    Button,
                    Node { padding: UiRect::axes(px(14), px(7)), flex_shrink: 0.0, ..default() },
                    BackgroundColor(Color::srgb(0.15, 0.28, 0.34)),
                ))
                .with_children(|button| {
                    button.spawn((Text::new("Reset"), text_font(14.0), TextColor(Color::srgb(0.88, 0.94, 0.96))));
                });
        });
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy queries select distinct UI components.
pub(crate) fn update(
    bridge: Res<Bridge>,
    windows: Query<&Window>,
    mut bars: Query<&mut Node, (With<Bar>, Without<Footer>, Without<Title>)>,
    mut footers: Query<&mut Node, (With<Footer>, Without<Bar>, Without<Title>)>,
    mut descriptions: Query<&mut Text, (With<Description>, Without<Help>, Without<Title>)>,
    mut help: Query<&mut Text, (With<Help>, Without<Description>, Without<Title>)>,
    mut help_fonts: Query<&mut TextFont, With<Help>>,
    mut titles: Query<(&mut Text, &mut Node), (With<Title>, Without<Bar>, Without<Footer>, Without<Description>, Without<Help>)>,
    mut buttons: Query<(Ref<Interaction>, &mut BackgroundColor), With<Reset>>,
) {
    let state = bridge.snapshot();
    let width = windows.iter().next().map_or(1280.0, |w| w.width());
    for mut node in &mut bars {
        node.height = px(height(width));
        if state.dashboard.as_ref().is_some_and(|data| !data.hash) {
            node.top = px(0);
            node.bottom = Val::Auto;
        } else {
            node.top = Val::Auto;
            node.bottom = px(footer_height(width));
        }
    }
    for mut node in &mut footers {
        node.height = px(footer_height(width));
    }
    let description = if let Some(data) = &state.dashboard {
        if data.hash {
            "Target is the intended number of events submitted per second. Confirmed / sec is the recent rate of events whose history updates have completed. Outstanding events are not yet confirmed. Workload memory is the memory used by the workload process. Each history uses a chain of hashes to check event order. If an event is applied in the wrong place anywhere in the chain, the hash will not match."
        } else {
            "Events update independent counters. The display shows submitted and confirmed work while ConTime processes the stream."
        }
    } else {
        "Each bot follows a square path, but its direction changes can arrive late or out of order. On the left, ConTime uses the original timestamps to correct the paths as inputs arrive. On the right, inputs take effect when they arrive, so the paths gradually drift away from their intended squares."
    };
    let help_text = if state.dashboard.as_ref().is_some_and(|data| data.hash) {
        "Inserted is an event accepted into a history. One event can reach multiple histories. Inserted / sec is the average rate since the first worker reading. Pending histories are scheduled for replay, including future work.\nThe left scale shows both events per second and the count of outstanding events. The right scale shows workload memory. Hover to inspect, click to pin, and click outside to release. Verified counts configured histories whose latest hashes match, including empty histories. A mismatch stops the run. Use arrow keys to browse workers and P to pause in the browser."
    } else if state.dashboard.is_some() {
        "Hover over the graph to inspect a reading. Use the arrow keys or Page Up and Page Down to browse workers. Press P to pause in the browser."
    } else {
        "Vertical lines mark one second. Gray dashes show intended routes, solid lines show current paths, and colored dashes show replaced paths. Double ticks mark inputs that arrived out of order.\nArrows show direction and dots mean stop. Move with WASD or the arrow keys. Press Space to stop or P to pause in the browser."
    };
    for (mut text, mut node) in &mut titles {
        if state.dashboard.as_ref().is_some_and(|data| !data.hash) {
            text.0 = "Counter".to_owned();
            node.display = Display::Flex;
        } else {
            node.display = Display::None;
        }
    }
    for mut text in &mut descriptions {
        if text.0 != description {
            text.0 = description.to_owned();
        }
    }
    for mut font in &mut help_fonts {
        font.font_size = if width < 600.0 && state.dashboard.as_ref().is_some_and(|data| data.hash) { 11.0.into() } else { 14.0.into() };
    }
    for mut text in &mut help {
        if text.0 != help_text {
            text.0 = help_text.to_owned();
        }
    }
    for (interaction, mut color) in &mut buttons {
        color.0 = if *interaction == Interaction::Hovered { Color::srgb(0.22, 0.40, 0.46) } else { Color::srgb(0.15, 0.28, 0.34) };
        if interaction.is_changed() && *interaction == Interaction::Pressed {
            bridge.request_reset();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counter_keeps_its_title_in_the_shared_header() {
        let bridge = Bridge::default();
        let report = serde_json::json!({"produced": 0, "completed": 0, "outstanding": 0, "queries": 0, "elapsed": 0.0, "workers": 1, "snapshot_count": 1});
        bridge.publish_dashboard(crate::dashboard::Dashboard::browser(&report, "counter").unwrap());
        let mut app = App::new();
        app.insert_resource(bridge).init_resource::<Assets<Font>>().add_systems(Startup, setup).add_systems(Update, update);
        app.update();
        let world = app.world_mut();
        let (text, node) = world.query_filtered::<(&Text, &Node), With<Title>>().single(world).unwrap();
        assert_eq!(text.0, "Counter");
        assert_eq!(node.display, Display::Flex);
    }
    #[test]
    fn benchmark_explains_verification_at_the_bottom_without_a_title() {
        let bridge = Bridge::default();
        let report = serde_json::json!({"produced": 0, "completed": 0, "outstanding": 0, "queries": 0, "elapsed": 0.0, "workers": 1, "snapshot_count": 1});
        bridge.publish_dashboard(crate::dashboard::Dashboard::browser(&report, "chain-hash").unwrap());
        let mut app = App::new();
        app.insert_resource(bridge).init_resource::<Assets<Font>>().add_systems(Startup, setup).add_systems(Update, update);
        app.update();
        let world = app.world_mut();
        let bar = world.query_filtered::<&Node, With<Bar>>().single(world).unwrap();
        assert_eq!(bar.top, Val::Auto);
        assert_eq!(bar.bottom, px(footer_height(1280.0)));
        assert_eq!(world.query_filtered::<&Node, With<Title>>().single(world).unwrap().display, Display::None);
        let description = world.query_filtered::<&Text, With<Description>>().single(world).unwrap().0.clone();
        assert!(description.contains("chain of hashes"));
        assert!(description.contains("order"));
        let help = &world.query_filtered::<&Text, With<Help>>().single(world).unwrap().0;
        assert!(help.contains("Verified"));
        assert!(help.contains("mismatch"));
        for term in ["Inserted / sec", "first worker reading", "Pending histories", "future"] {
            assert!(help.contains(term), "missing {term}");
        }
        for term in ["Confirmed", "Outstanding", "Target", "Workload memory"] {
            assert!(description.contains(term), "missing {term}");
        }
    }
    #[test]
    fn game_has_a_single_explanation_and_a_separate_footer_with_reset() {
        let bridge = Bridge::default();
        let mut app = App::new();
        app.insert_resource(bridge.clone()).init_resource::<Assets<Font>>().add_systems(Startup, setup).add_systems(Update, update);
        app.update();
        {
            let world = app.world_mut();
            let text = world.query_filtered::<&Text, With<Description>>().single(world).unwrap();
            assert_eq!(text.0.matches('\n').count(), 0);
            assert!(text.0.contains("On the left, ConTime"));
            assert!(!text.0.contains("Gray dashes"));
            let bar = world.query_filtered::<&Node, With<Bar>>().single(world).unwrap();
            assert_eq!(bar.top, Val::Auto);
            assert_eq!(bar.bottom, px(footer_height(1280.0)));
            assert_eq!(world.query_filtered::<&Node, With<Title>>().single(world).unwrap().display, Display::None);
            let footer = world.query_filtered::<&Text, With<Help>>().single(world).unwrap();
            assert!(footer.0.contains("Gray dashes"));
            *world.query_filtered::<&mut Interaction, With<Reset>>().single_mut(world).unwrap() = Interaction::Pressed;
        }
        app.update();
        assert!(bridge.take_reset());
        assert!(!bridge.take_reset());
        app.update();
        assert!(!bridge.take_reset());
    }
}
