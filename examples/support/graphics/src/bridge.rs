use crate::{
    catalog::{Navigation, Visualization},
    report::Report,
};
use bevy::prelude::Resource;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct State {
    pub choosing: bool,
    pub transitioning: bool,
    pub can_choose: bool,
    navigation: Option<Navigation>,
    pub selected: Option<crate::catalog::Visualization>,
    selection: Option<crate::catalog::Visualization>,
    pub report: Option<Arc<Report>>,
    pub uptime: Option<Arc<crate::uptime_report::Report>>,
    pub accounts: Option<Arc<crate::accounts_report::Report>>,
    pub robotics: Option<Arc<crate::robotics_report::Report>>,
    pub running: bool,
    pub resume_epoch: u64,
    pub finished: bool,
    pub reduced_motion: bool,
    pub generation: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            choosing: false,
            transitioning: false,
            can_choose: true,
            navigation: None,
            selected: None,
            selection: None,
            report: None,
            uptime: None,
            accounts: None,
            robotics: None,
            running: false,
            resume_epoch: 0,
            finished: false,
            reduced_motion: false,
            generation: 0,
        }
    }
}
impl State {
    pub fn active(&self) -> Option<Visualization> {
        self.selected.or_else(|| {
            if self.robotics.is_some() {
                Some(Visualization::Robotics)
            } else if self.accounts.is_some() {
                Some(Visualization::Accounts)
            } else if self.uptime.is_some() {
                Some(Visualization::Uptime)
            } else if self.report.is_some() {
                Some(Visualization::Movement)
            } else {
                None
            }
        })
    }
}

/// A latest-value slot: presentation never queues or processes application events.
#[derive(Clone, Default, Resource)]
pub struct Bridge(Arc<Mutex<State>>);
impl Bridge {
    pub fn can_choose(&self, enabled: bool) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).can_choose = enabled;
    }
    pub fn request_navigation(&self, navigation: Navigation) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.transitioning || state.choosing || state.active().is_none() || (navigation == Navigation::Menu && !state.can_choose) {
            return;
        }
        state.navigation = Some(navigation);
        state.transitioning = true;
    }
    pub fn take_navigation(&self) -> Option<Navigation> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).navigation.take()
    }
    pub fn restart_view(&self) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let selected = state.active();
        *state = State {
            selected,
            can_choose: state.can_choose,
            reduced_motion: state.reduced_motion,
            generation: state.generation.wrapping_add(1),
            ..State::default()
        };
    }
    pub fn show_chooser(&self) {
        self.reset();
        self.0.lock().unwrap_or_else(|p| p.into_inner()).choosing = true;
    }
    pub fn select(&self, visualization: crate::catalog::Visualization) {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.choosing {
            state.choosing = false;
            state.selected = Some(visualization);
            state.selection = Some(visualization);
        }
    }
    pub fn take_selection(&self) -> Option<crate::catalog::Visualization> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).selection.take()
    }

    pub fn publish(&self, report: Report) -> Result<(), String> {
        report.validate()?;
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.uptime.is_some() || state.accounts.is_some() || state.robotics.is_some() {
            state.generation = state.generation.wrapping_add(1);
        }
        state.uptime = None;
        state.accounts = None;
        state.robotics = None;
        state.selected = Some(Visualization::Movement);
        state.report = Some(Arc::new(report));
        Ok(())
    }
    pub fn publish_uptime(&self, report: crate::uptime_report::Report) -> Result<(), String> {
        report.validate()?;
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.report.is_some() || state.accounts.is_some() || state.robotics.is_some() {
            state.generation = state.generation.wrapping_add(1);
        }
        state.report = None;
        state.accounts = None;
        state.robotics = None;
        state.selected = Some(Visualization::Uptime);
        state.uptime = Some(Arc::new(report));
        Ok(())
    }
    pub fn publish_accounts(&self, report: crate::accounts_report::Report) -> Result<(), String> {
        report.validate()?;
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.report.is_some() || state.uptime.is_some() || state.robotics.is_some() {
            state.generation = state.generation.wrapping_add(1);
        }
        state.report = None;
        state.uptime = None;
        state.robotics = None;
        state.selected = Some(Visualization::Accounts);
        state.accounts = Some(Arc::new(report));
        Ok(())
    }
    pub fn publish_robotics(&self, report: crate::robotics_report::Report) -> Result<(), String> {
        report.validate()?;
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.report.is_some() || state.uptime.is_some() || state.accounts.is_some() {
            state.generation = state.generation.wrapping_add(1);
        }
        state.report = None;
        state.uptime = None;
        state.accounts = None;
        state.selected = Some(Visualization::Robotics);
        state.robotics = Some(Arc::new(report));
        Ok(())
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
    fn navigation_is_one_shot_while_paused_or_finished_and_restart_preserves_identity() {
        for finished in [false, true] {
            let bridge = Bridge::default();
            bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
            bridge.reduced_motion(true);
            if finished {
                bridge.finish();
            }
            bridge.request_navigation(Navigation::Reset);
            bridge.request_navigation(Navigation::Menu);
            assert_eq!(bridge.take_navigation(), Some(Navigation::Reset));
            assert_eq!(bridge.take_navigation(), None);
            bridge.request_navigation(Navigation::Menu);
            assert_eq!(bridge.take_navigation(), None);
            let generation = bridge.snapshot().generation;
            bridge.restart_view();
            let state = bridge.snapshot();
            assert_eq!(state.active(), Some(Visualization::Uptime));
            assert!(state.uptime.is_none() && state.report.is_none());
            assert!(state.reduced_motion && !state.transitioning && !state.finished);
            assert_ne!(state.generation, generation);
            bridge.request_navigation(Navigation::Menu);
            assert_eq!(bridge.take_navigation(), Some(Navigation::Menu));
            bridge.show_chooser();
            assert!(bridge.snapshot().choosing);
            assert!(bridge.snapshot().active().is_none());
        }
    }
    #[test]
    fn direct_mode_hides_menu_but_allows_reset() {
        let bridge = Bridge::default();
        bridge.can_choose(false);
        bridge.publish(scene(1.0)).unwrap();
        bridge.request_navigation(Navigation::Menu);
        assert_eq!(bridge.take_navigation(), None);
        bridge.request_navigation(Navigation::Reset);
        assert_eq!(bridge.take_navigation(), Some(Navigation::Reset));
        bridge.restart_view();
        assert!(!bridge.snapshot().can_choose);
    }
    #[test]
    fn robotics_is_exclusive_and_reset_preserves_the_new_choice() {
        let bridge = Bridge::default();
        bridge.publish(scene(0.0)).unwrap();
        bridge.publish_robotics(crate::robotics_report::Report::default()).unwrap();
        assert!(bridge.snapshot().report.is_none());
        assert_eq!(bridge.snapshot().active(), Some(Visualization::Robotics));
        bridge.request_navigation(Navigation::Reset);
        assert_eq!(bridge.take_navigation(), Some(Navigation::Reset));
        bridge.restart_view();
        assert_eq!(bridge.snapshot().active(), Some(Visualization::Robotics));
        bridge.show_chooser();
        bridge.select(Visualization::Robotics);
        assert_eq!(bridge.take_selection(), Some(Visualization::Robotics));
        bridge.publish_robotics(crate::robotics_report::Report::default()).unwrap();
        bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
        assert!(bridge.snapshot().robotics.is_none());
    }
    #[test]
    fn accounts_are_exclusive_and_keep_reset_and_menu_navigation() {
        let bridge = Bridge::default();
        bridge.publish(scene(0.0)).unwrap();
        bridge.publish_accounts(crate::accounts_report::Report::default()).unwrap();
        let state = bridge.snapshot();
        assert!(state.report.is_none() && state.uptime.is_none() && state.accounts.is_some());
        assert_eq!(state.active(), Some(Visualization::Accounts));
        bridge.request_navigation(Navigation::Reset);
        assert_eq!(bridge.take_navigation(), Some(Navigation::Reset));
        bridge.restart_view();
        assert_eq!(bridge.snapshot().active(), Some(Visualization::Accounts));
        bridge.show_chooser();
        assert!(bridge.snapshot().accounts.is_none());
        bridge.select(Visualization::Accounts);
        assert_eq!(bridge.take_selection(), Some(Visualization::Accounts));
        bridge.publish_accounts(crate::accounts_report::Report::default()).unwrap();
        bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
        assert!(bridge.snapshot().accounts.is_none());
    }
    #[test]
    fn chooser_is_one_shot_and_independent_of_reports() {
        let bridge = Bridge::default();
        bridge.show_chooser();
        assert!(bridge.snapshot().choosing);
        assert!(bridge.snapshot().report.is_none());
        bridge.select(crate::catalog::Visualization::Uptime);
        bridge.select(crate::catalog::Visualization::Movement);
        assert_eq!(bridge.take_selection(), Some(crate::catalog::Visualization::Uptime));
        assert_eq!(bridge.take_selection(), None);
        assert_eq!(bridge.snapshot().selected, Some(crate::catalog::Visualization::Uptime));
        bridge.show_chooser();
        assert_eq!(bridge.take_selection(), None);
        assert!(bridge.snapshot().choosing);
    }
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
    fn uptime_switches_exclusively_rejects_invalid_reports_and_resets() {
        let bridge = Bridge::default();
        bridge.publish(scene(1.0)).unwrap();
        let before = bridge.snapshot().generation;
        let uptime = crate::uptime_report::Report { window_start_ms: 1, ..Default::default() };
        assert!(bridge.publish_uptime(uptime).is_err());
        assert!(bridge.snapshot().report.is_some());
        bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
        let state = bridge.snapshot();
        assert!(state.report.is_none() && state.uptime.is_some());
        assert_ne!(state.generation, before);
        bridge.running(true);
        bridge.finish();
        assert!(!bridge.snapshot().running && bridge.snapshot().finished);
        bridge.publish(scene(2.0)).unwrap();
        assert!(bridge.snapshot().uptime.is_none());
        bridge.publish_uptime(crate::uptime_report::Report::default()).unwrap();
        bridge.reset();
        let state = bridge.snapshot();
        assert!(state.uptime.is_none() && state.report.is_none() && !state.finished);
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
