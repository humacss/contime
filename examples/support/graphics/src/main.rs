use bevy::{
    app::AppExit,
    prelude::*,
    window::{PrimaryWindow, WindowCloseRequested},
};
use contime_graphics_example::{
    bridge::Bridge,
    catalog::{Catalog, Launch, Navigation, Visualization},
    native::Source,
    scene::{self, GraphicsSet},
};
use std::{io, process::Command};

struct Desktop {
    source: Option<Source>,
    catalog: Option<Catalog>,
    active: Option<Launch>,
    bridge: Bridge,
    transition: Option<Navigation>,
    stopping: bool,
    closing: bool,
}
impl Desktop {
    fn start(&mut self, launch: Launch) -> Result<(), String> {
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).current_dir(&launch.cwd);
        self.source = Some(Source::spawn(command).map_err(|error| error.to_string())?);
        self.active = Some(launch);
        self.stopping = false;
        self.bridge.running(true);
        Ok(())
    }
    fn tick(&mut self, quitting: bool, input: &[u8]) -> Result<bool, String> {
        self.closing |= quitting;
        if let Some(navigation) = self.bridge.take_navigation() {
            self.transition = Some(navigation);
        }
        if self.source.is_none() {
            if self.closing {
                return Ok(true);
            }
            if let Some(navigation) = self.transition.take() {
                match navigation {
                    Navigation::Reset => {
                        let launch = self.active.clone().ok_or("Missing current workload command")?;
                        self.bridge.restart_view();
                        self.start(launch)?;
                    }
                    Navigation::Menu => {
                        self.active = None;
                        self.bridge.show_chooser();
                    }
                }
                return Ok(false);
            }
            if let Some(selected) = self.bridge.take_selection() {
                let launch = self
                    .catalog
                    .as_ref()
                    .ok_or("Missing visualization catalog")?
                    .launches
                    .iter()
                    .find(|launch| launch.visualization == selected)
                    .cloned()
                    .ok_or("Missing selected workload")?;
                self.start(launch)?;
            }
            // Selection keys never reach the newly started workload.
            return Ok(false);
        }
        let source = self.source.as_mut().unwrap();
        if !self.stopping && (self.closing || self.transition.is_some()) {
            source.stop().map_err(|error| error.to_string())?;
            self.stopping = true;
        }
        if !self.stopping {
            for key in input {
                source.input(*key).map_err(|error| error.to_string())?;
            }
        }
        if source.poll(&self.bridge)? {
            let state = self.bridge.snapshot();
            if let Some(report) = state.uptime {
                println!(
                    "Validated {} service changes; {} late arrivals; {} completed queries",
                    report.submitted, report.late, report.updates
                );
            }
            if let Some(report) = state.report {
                println!(
                    "Validated {} delivered inputs; {} reordered arrivals; {} corrected histories; {} keyboard inputs",
                    report.submitted, report.late, report.corrections, report.keyboard_inputs
                );
            }
            self.source.take(); // The old process has validated and exited successfully before replacement.
            self.stopping = false;
            self.bridge.finish();
            return Ok(self.closing);
        }
        Ok(false)
    }
}
fn desktop(
    mut desktop: NonSendMut<Desktop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut close: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let quitting = close.read().next().is_some() || keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyQ);
    let input: Vec<u8> = [
        (KeyCode::KeyW, b'w'),
        (KeyCode::ArrowUp, b'w'),
        (KeyCode::KeyS, b's'),
        (KeyCode::ArrowDown, b's'),
        (KeyCode::KeyA, b'a'),
        (KeyCode::ArrowLeft, b'a'),
        (KeyCode::KeyD, b'd'),
        (KeyCode::ArrowRight, b'd'),
        (KeyCode::Space, b' '),
    ]
    .into_iter()
    .filter_map(|(key, value)| keys.just_pressed(key).then_some(value))
    .collect();
    let result = desktop.tick(quitting, &input);
    if let Ok(mut window) = windows.single_mut() {
        let state = desktop.bridge.snapshot();
        window.title = if desktop.stopping {
            "ConTime / Finishing validation".into()
        } else if let Some(active) = state.active() {
            format!("ConTime / {}", active.label())
        } else {
            "ConTime / Choose a visualization".into()
        };
    }
    match result {
        Ok(true) => {
            desktop.bridge.finish();
            exit.write(AppExit::Success);
        }
        Err(error) => {
            eprintln!("Visualization workload failed: {error}");
            exit.write(AppExit::error());
        }
        Ok(false) => {}
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let program = args.next().ok_or_else(|| io::Error::other("Pass a workload executable followed by its arguments"))?;
    let bridge = Bridge::default();
    let mut desktop =
        Desktop { source: None, catalog: None, active: None, bridge: bridge.clone(), transition: None, stopping: false, closing: false };
    if program == "--visualizer-catalog" {
        let json = args.next().ok_or_else(|| io::Error::other("Missing visualization catalog"))?;
        if args.next().is_some() {
            return Err(io::Error::other("Unexpected arguments after visualization catalog").into());
        }
        let catalog: Catalog = serde_json::from_str(&json.into_string().map_err(|_| io::Error::other("Catalog must be UTF-8 JSON"))?)?;
        catalog.validate().map_err(io::Error::other)?;
        bridge.show_chooser();
        desktop.catalog = Some(catalog);
    } else {
        bridge.can_choose(false);
        desktop
            .start(Launch { visualization: Visualization::Movement, program, args: args.collect(), cwd: std::env::current_dir()? })
            .map_err(io::Error::other)?;
    }
    let mut app = scene::app(bridge, None);
    app.insert_non_send(desktop).add_systems(Update, crate::desktop.in_set(GraphicsSet::Collect));
    if app.run().is_success() { Ok(()) } else { Err(io::Error::other("Graphical example failed").into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn waiting() -> Desktop {
        let bridge = Bridge::default();
        bridge.show_chooser();
        Desktop { source: None, catalog: None, active: None, bridge, transition: None, stopping: false, closing: false }
    }
    #[test]
    fn chooser_waits_and_cancels_without_spawning_a_workload() {
        let mut desktop = waiting();
        assert!(!desktop.tick(false, &[]).unwrap());
        assert!(desktop.source.is_none());
        assert!(desktop.tick(true, &[]).unwrap());
        assert!(desktop.source.is_none());
    }
    #[cfg(unix)]
    fn launch() -> Launch {
        let payload = serde_json::json!({"done": true, "uptime": contime_graphics_example::uptime_report::Report::default()});
        Launch {
            visualization: Visualization::Uptime,
            program: "sh".into(),
            args: vec![
                "-c".into(),
                "dd bs=1 count=1 >/dev/null 2>/dev/null; printf '%s\\n' \"$1\"".into(),
                "fixture".into(),
                format!("contime-workers-v2\t{payload}").into(),
            ],
            cwd: std::env::current_dir().unwrap(),
        }
    }
    #[cfg(unix)]
    fn drain(desktop: &mut Desktop) {
        let start = std::time::Instant::now();
        while desktop.source.is_some() {
            desktop.tick(false, &[]).unwrap();
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    #[cfg(unix)]
    fn reset_waits_for_validation_then_reuses_command_and_menu_reaps_the_replacement() {
        let mut desktop = waiting();
        desktop.bridge.select(Visualization::Uptime);
        desktop.bridge.take_selection();
        desktop.start(launch()).unwrap();
        let expected_args = desktop.active.as_ref().unwrap().args.clone();
        desktop.bridge.request_navigation(Navigation::Reset);
        drain(&mut desktop);
        assert!(desktop.bridge.snapshot().finished);
        assert_eq!(desktop.transition, Some(Navigation::Reset));
        desktop.tick(false, &[]).unwrap();
        assert!(desktop.source.is_some());
        assert_eq!(desktop.active.as_ref().unwrap().args, expected_args);
        assert!(desktop.bridge.snapshot().uptime.is_none());
        desktop.bridge.request_navigation(Navigation::Menu);
        drain(&mut desktop);
        desktop.tick(false, &[]).unwrap();
        assert!(desktop.source.is_none());
        assert!(desktop.bridge.snapshot().choosing);
        assert!(desktop.active.is_none());
    }
    #[test]
    #[cfg(unix)]
    fn finite_completion_before_navigation_allows_reset_menu_and_close() {
        for action in [Some(Navigation::Reset), Some(Navigation::Menu), None] {
            let mut desktop = waiting();
            desktop.bridge.select(Visualization::Uptime);
            desktop.bridge.take_selection();
            let mut finite = launch();
            finite.args[1] = "printf '%s\\n' \"$1\"".into();
            desktop.start(finite).unwrap();
            let start = std::time::Instant::now();
            // Wait for the finite child to close stdin without polling its final
            // report. Native transport tests additionally confirm actual exit.
            loop {
                match desktop.source.as_mut().unwrap().input(b' ') {
                    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => break,
                    Err(error) => panic!("Unexpected input error: {error}"),
                    Ok(()) => {}
                }
                assert!(start.elapsed() < std::time::Duration::from_secs(3));
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            if let Some(action) = action {
                desktop.bridge.request_navigation(action);
            }
            desktop.tick(action.is_none(), &[]).unwrap();
            drain(&mut desktop);
            assert!(desktop.bridge.snapshot().finished);
            let closing = desktop.tick(false, &[]).unwrap();
            match action {
                Some(Navigation::Reset) => {
                    assert!(!closing);
                    assert!(desktop.source.is_some());
                    drain(&mut desktop);
                    assert!(desktop.bridge.snapshot().finished);
                }
                Some(Navigation::Menu) => {
                    assert!(!closing);
                    assert!(desktop.bridge.snapshot().choosing);
                }
                None => assert!(closing),
            }
        }
    }
    #[test]
    #[cfg(unix)]
    fn close_while_draining_exits_after_validation_without_restarting() {
        let mut desktop = waiting();
        desktop.bridge.select(Visualization::Uptime);
        desktop.bridge.take_selection();
        desktop.start(launch()).unwrap();
        desktop.bridge.request_navigation(Navigation::Reset);
        desktop.tick(true, &[]).unwrap();
        drain(&mut desktop);
        assert!(desktop.bridge.snapshot().finished);
        assert!(desktop.tick(false, &[]).unwrap());
        assert!(desktop.source.is_none());
    }
}
