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
impl Scene {
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
        while self.ghosts.len() > 32 {
            self.ghosts.pop_front();
        }
    }
    /// Old points are revisions only when the latest query disagrees for the
    /// same client at the same time. Sliding-window endpoints are not comparable.
    pub fn revised_positions<'a>(&'a self, old: &'a Rendering) -> impl Iterator<Item = &'a [i128; 2]> + 'a {
        old.samples.iter().flat_map(move |sample| {
            let current = self.current.samples.iter().find(|next| next.time == sample.time);
            sample.positions.iter().enumerate().filter_map(move |(client, position)| {
                current.and_then(|next| next.positions.get(client)).filter(|next| *next != position).map(|_| position)
            })
        })
    }
    pub fn expire(&mut self, now: u64, fade: u64) {
        self.ghosts.retain(|ghost| now.saturating_sub(ghost.created) < fade);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn state(points: &[(u64, i128)]) -> Rendering {
        Rendering { samples: points.iter().map(|&(t, x)| Sample { time: Time(t), positions: vec![[x, 0]] }).collect() }
    }
    #[test]
    fn normal_motion_is_not_a_correction_but_revised_history_fades_as_a_ghost() {
        let mut scene = Scene::default();
        scene.publish(state(&[(1, 0)]), 0);
        scene.publish(state(&[(1, 0), (2, 1)]), 10);
        assert_eq!(scene.corrections, 0);
        scene.publish(state(&[(1, 0), (2, 2), (3, 3)]), 20);
        assert_eq!(scene.corrections, 1);
        assert_eq!(scene.revised_positions(&scene.ghosts.back().unwrap().state).count(), 1);
        assert_eq!(scene.ghosts.back().unwrap().state.samples[1].positions[0], [1, 0]);
        scene.expire(100, 50);
        assert!(scene.ghosts.is_empty());
    }
    #[test]
    fn faded_paths_only_include_clients_that_diverge_at_matching_times() {
        let mut scene = Scene::default();
        let old = Rendering {
            samples: vec![
                Sample { time: Time(10), positions: vec![[10, 0], [0, 10]] },
                Sample { time: Time(15), positions: vec![[15, 0], [0, 15]] },
                Sample { time: Time(20), positions: vec![[20, 0], [0, 20]] },
            ],
        };
        scene.publish(old, 0);
        scene.publish(
            Rendering {
                samples: vec![
                    Sample { time: Time(10), positions: vec![[10, 0], [1, 10]] },
                    Sample { time: Time(20), positions: vec![[20, 0], [2, 20]] },
                    Sample { time: Time(30), positions: vec![[30, 0], [3, 30]] },
                ],
            },
            10,
        );
        let revised: Vec<_> = scene.revised_positions(&scene.ghosts[0].state).copied().collect();
        // Client 1 is moving straight without a correction; its old endpoint
        // at t=15 is not proof of a correction either. Only client 2 changed.
        assert_eq!(revised, vec![[0, 10], [0, 20]]);
        scene.publish(scene.current.clone(), 20);
        assert_eq!(scene.revised_positions(&scene.ghosts[1].state).count(), 0);
    }
}
