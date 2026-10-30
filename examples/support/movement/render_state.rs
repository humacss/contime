//! Query-owned rendering states and fading copies; no predicted character state.
use crate::movement_example::model::Time;
use std::collections::{BTreeSet, VecDeque};
#[derive(Clone, Default)]
pub struct Sample {
    pub time: Time,
    pub positions: Vec<[i128; 2]>,
}
#[derive(Clone, Default)]
pub struct Rendering {
    pub directions: Vec<[f64; 2]>,
    pub samples: Vec<Sample>,
}
pub struct Ghost {
    pub state: Rendering,
    pub created: u64,
}
#[derive(Default)]
pub struct Scene {
    pub current: Rendering,
    pub ghosts: VecDeque<Ghost>,
    pub corrected: BTreeSet<usize>,
    pub corrections: u64,
}
/// One client segment: client ID, endpoint time, start and end positions.
pub type Segment = (usize, u64, [i128; 2], [i128; 2]);
impl Scene {
    /// Connect only adjacent samples of the same client. A revision requires
    /// comparable endpoints; a sliding trail window alone proves no correction.
    pub fn segments<'a>(&'a self, state: &'a Rendering, revised_only: bool) -> impl Iterator<Item = Segment> + 'a {
        state.samples.windows(2).flat_map(move |pair| {
            let start = &pair[0];
            let end = &pair[1];
            let matching =
                self.current.samples.iter().find(|s| s.time == start.time).zip(self.current.samples.iter().find(|s| s.time == end.time));
            start.positions.iter().zip(&end.positions).enumerate().filter_map(move |(client, (&from, &to))| {
                let changed = matching.is_some_and(|(a, b)| {
                    a.positions.get(client).zip(b.positions.get(client)).is_some_and(|(x, y)| *x != from || *y != to)
                });
                (!revised_only || changed).then_some((client + 1, end.time.0, from, to))
            })
        })
    }
    pub fn publish(&mut self, next: Rendering, now: u64) {
        self.corrected.clear();
        for old in &self.current.samples {
            if let Some(new) = next.samples.iter().find(|sample| sample.time == old.time) {
                for (client, (a, b)) in old.positions.iter().zip(&new.positions).enumerate() {
                    if a != b {
                        self.corrected.insert(client);
                    }
                }
            }
        }
        self.corrections += self.corrected.len() as u64;
        let old = std::mem::replace(&mut self.current, next);
        if !old.samples.is_empty() {
            self.ghosts.push_back(Ghost { state: old, created: now });
        }
        // Cover the maximum five-second fade at up to 60 query updates/s.
        while self.ghosts.len() > 320 {
            self.ghosts.pop_front();
        }
    }
    pub fn expire(&mut self, now: u64, fade: u64) {
        self.ghosts.retain(|ghost| now.saturating_sub(ghost.created) < fade);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state(points: &[(u64, i128)]) -> Rendering {
        Rendering { directions: vec![], samples: points.iter().map(|&(t, x)| Sample { time: Time(t), positions: vec![[x, 0]] }).collect() }
    }
    #[test]
    fn normal_motion_is_not_a_correction_but_revised_history_fades_as_a_ghost() {
        let mut scene = Scene::default();
        scene.publish(state(&[(1, 0)]), 0);
        scene.publish(state(&[(1, 0), (2, 1)]), 10);
        assert_eq!(scene.corrections, 0);
        scene.publish(state(&[(1, 0), (2, 2), (3, 3)]), 20);
        assert_eq!(scene.corrections, 1);
        assert_eq!(scene.segments(&scene.ghosts.back().unwrap().state, true).count(), 1);
        assert_eq!(scene.ghosts.back().unwrap().state.samples[1].positions[0], [1, 0]);
        scene.expire(100, 50);
        assert!(scene.ghosts.is_empty());
    }
    #[test]
    fn faded_paths_only_include_clients_that_diverge_at_matching_times() {
        let mut scene = Scene::default();
        let old = Rendering {
            directions: vec![],
            samples: vec![
                Sample { time: Time(10), positions: vec![[10, 0], [0, 10]] },
                Sample { time: Time(15), positions: vec![[15, 0], [0, 15]] },
                Sample { time: Time(20), positions: vec![[20, 0], [0, 20]] },
                Sample { time: Time(30), positions: vec![[30, 0], [0, 30]] },
            ],
        };
        scene.publish(old, 0);
        scene.publish(
            Rendering {
                directions: vec![],
                samples: vec![
                    Sample { time: Time(10), positions: vec![[10, 0], [1, 10]] },
                    Sample { time: Time(20), positions: vec![[20, 0], [2, 20]] },
                    Sample { time: Time(30), positions: vec![[30, 0], [3, 30]] },
                    Sample { time: Time(40), positions: vec![[40, 0], [4, 40]] },
                ],
            },
            10,
        );
        let revised: Vec<_> = scene.segments(&scene.ghosts[0].state, true).collect();
        // Client 1 is moving straight without a correction; its old endpoint
        // at t=15 is not proof of a correction either. Only the comparable
        // t=20 to t=30 segment for client 2 is a visible revision.
        assert_eq!(revised, vec![(2, 30, [0, 20], [0, 30])]);
        scene.publish(scene.current.clone(), 20);
        assert_eq!(scene.segments(&scene.ghosts[1].state, true).count(), 0);
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    fn rendering(points: &[(u64, i128)]) -> Rendering {
        Rendering {
            directions: vec![],
            samples: points.iter().map(|&(time, x)| Sample { time: Time(time), positions: vec![[x, 0], [0, x]] }).collect(),
        }
    }
    #[test]
    fn segments_keep_clients_separate_and_include_the_branch_origin() {
        let old = rendering(&[(1, 0), (2, 1), (3, 2)]);
        let scene = Scene { current: rendering(&[(1, 0), (2, 5), (3, 6)]), ..Scene::default() };
        let segments: Vec<_> = scene.segments(&old, true).collect();
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[0], (1, 2, [0, 0], [1, 0]));
        assert_eq!(segments[1], (2, 2, [0, 0], [0, 1]));
        assert!(scene.segments(&scene.current, true).next().is_none());
        assert_eq!(scene.segments(&scene.current, false).count(), 4);
    }
    #[test]
    fn revisions_survive_a_full_five_second_fade_at_sixty_queries_per_second() {
        let mut scene = Scene::default();
        for tick in 0..=300 {
            scene.publish(rendering(&[(1, tick), (2, tick + 1)]), tick as u64 * 16_666_666);
        }
        scene.expire(5_000_000_000, 5_000_000_000);
        assert!(scene.ghosts.front().unwrap().created < 100_000_000, "the frame-count cap must not shorten the configured fade");
    }
    #[test]
    fn sliding_window_endpoints_are_not_revisions() {
        let scene = Scene { current: rendering(&[(2, 1), (3, 2)]), ..Scene::default() };
        assert!(scene.segments(&rendering(&[(1, 0), (2, 1)]), true).next().is_none());
    }
}
