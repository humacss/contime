//! Static intended paths use the same inputs and motion rules as the example.
//! These guides are computed at startup, never from delivered or queried state.
use super::{generation, showcase, Config};
use crate::{
    movement_example::model::{Character, Motion, SECOND},
    support::movement_report::IntendedPath,
};

pub(super) fn paths(config: &Config, scripted: bool) -> Vec<IntendedPath> {
    let motion = Motion { client_spacing: config.client_spacing, movement_speed: config.movement_speed };
    (1..=config.clients)
        // Free keyboard movement has no predetermined path.
        .filter(|&client| scripted || client != 1)
        .map(|client| {
            let mut state = Character::spawn(client as u128, motion);
            let turns = if client == 1 { 8 } else { 4 };
            let points = (0..=turns)
                .map(|turn| {
                    let event = if scripted {
                        showcase::input(turn * 5 + client as u64 - 1, motion).0
                    } else {
                        let time =
                            generation::scheduled(config.clients, config.input_hz, turn * (config.clients - 1) as u64 + client as u64 - 2)
                                .expect("bot guide has a configured client and frequency")
                                .1;
                        generation::event(client, turn + 1, time, motion, turn)
                    };
                    state.apply_input(&event);
                    state.position.map(|coordinate| coordinate as f64 / SECOND as f64)
                })
                .collect();
            IntendedPath { client, points }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(args: &[&str]) -> Config {
        Config::parse(args.iter().map(|value| value.to_string())).unwrap().unwrap()
    }
    #[test]
    fn interactive_guides_close_each_bot_square_and_follow_the_selected_scale() {
        for hz in [1, 2, 20] {
            let paths = paths(&config(&["--input-hz", &hz.to_string(), "--movement-speed", "12"]), false);
            assert_eq!(paths.len(), 4);
            for path in paths {
                assert_ne!(path.client, 1);
                assert_eq!(path.points.len(), 5);
                assert_eq!(path.points.first(), path.points.last());
                for segment in path.points.windows(2) {
                    let length: f64 = (0..2).map(|axis| (segment[1][axis] - segment[0][axis]).abs()).sum();
                    assert!((length - 12.0 / hz as f64).abs() < 0.000001);
                }
            }
        }
    }
    #[test]
    fn recording_alone_includes_the_players_exact_out_and_back_axes() {
        let paths = paths(&config(&["--client-spacing", "18", "--movement-speed", "8"]), true);
        assert_eq!(paths.len(), 5);
        assert_eq!(
            paths[0].points,
            vec![[0.0, 0.0], [0.0, 8.0], [0.0, 0.0], [8.0, 0.0], [0.0, 0.0], [-8.0, 0.0], [0.0, 0.0], [0.0, -8.0], [0.0, 0.0],]
        );
        for path in &paths[1..] {
            assert_eq!(path.points.first(), path.points.last());
        }
    }
    #[test]
    fn guides_respect_client_count_and_walls_at_extreme_settings() {
        assert!(paths(&config(&["--clients", "1"]), false).is_empty());
        for path in paths(&config(&["--movement-speed", "1000000", "--client-spacing", "1000000"]), false) {
            for point in path.points {
                assert!(point[0].abs() <= 48.0 && point[1].abs() <= 28.0);
            }
        }
    }
}
