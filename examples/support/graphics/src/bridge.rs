use crate::report::Report;
use bevy::prelude::Resource;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct State {
    pub report: Option<Arc<Report>>,
    pub dashboard: Option<Arc<crate::dashboard::Dashboard>>,
    pub running: bool,
    pub resume_epoch: u64,
    pub finished: bool,
    pub reduced_motion: bool,
    pub generation: u64,
    pub reset_requested: bool,
}

/// A latest-value slot: presentation never queues or processes application events.
#[derive(Clone, Default, Resource)]
pub struct Bridge(Arc<Mutex<State>>);
impl Bridge {
    pub fn publish(&self, report: Report) -> Result<(), String> {
        report.validate()?;
        self.0.lock().unwrap_or_else(|p| p.into_inner()).report = Some(Arc::new(report));
        Ok(())
    }
    pub fn publish_dashboard(&self, dashboard: crate::dashboard::Dashboard) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).dashboard = Some(Arc::new(dashboard));
    }
    pub fn snapshot(&self) -> State {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn running(&self, running: bool) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if running && !state.running {
            state.resume_epoch = state.resume_epoch.wrapping_add(1);
        }
        state.running = running;
    }
    pub fn reduced_motion(&self, reduced: bool) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).reduced_motion = reduced;
    }
    pub fn finish(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        state.running = false;
        state.finished = true;
    }
    pub fn request_reset(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).reset_requested = true;
    }
    pub fn take_reset(&self) -> bool {
        std::mem::take(&mut self.0.lock().unwrap_or_else(|p| p.into_inner()).reset_requested)
    }
    pub fn reset(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let generation = state.generation.wrapping_add(1);
        *state = State { generation, ..State::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Animation, tests::scene};
    #[test]
    fn completion_displays_exact_final_query_while_pause_preserves_animation_state() {
        let bridge = Bridge::default();
        let mut animation = Animation::default();
        animation.update(&scene(0.0), 0.0, false);
        bridge.publish(scene(10.0)).unwrap();
        bridge.finish();
        let state = bridge.snapshot();
        animation.update(state.report.as_deref().unwrap(), 0.0, state.reduced_motion || state.finished);
        assert_eq!(animation.clients[&1].position, [10.0, 0.0]);
        assert!(!state.running);
        bridge.reset();
        assert!(!bridge.snapshot().finished);
    }
    #[test]
    fn pause_and_resume_are_observable_even_without_a_paused_render_frame() {
        let bridge = Bridge::default();
        bridge.running(true);
        let first = bridge.snapshot().resume_epoch;
        bridge.running(true);
        assert_eq!(bridge.snapshot().resume_epoch, first);
        bridge.running(false);
        bridge.running(true);
        assert_ne!(bridge.snapshot().resume_epoch, first);
    }
}
