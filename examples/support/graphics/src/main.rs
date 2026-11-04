use bevy::{
    app::AppExit,
    prelude::*,
    window::{PrimaryWindow, WindowCloseRequested},
};
use contime_graphics_example::{
    bridge::Bridge,
    native::Source,
    scene::{self, GraphicsSet},
};
use std::{
    io,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Desktop {
    source: Source,
    bridge: Bridge,
    interrupted: Arc<AtomicBool>,
    stopping: bool,
    failed: bool,
    restarting: bool,
    program: std::ffi::OsString,
    args: Vec<std::ffi::OsString>,
}
fn desktop(
    mut desktop: NonSendMut<Desktop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut close: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let quitting = desktop.interrupted.load(Ordering::Acquire)
        || close.read().next().is_some()
        || keys.just_pressed(KeyCode::Escape)
        || keys.just_pressed(KeyCode::KeyQ);
    if desktop.failed {
        if quitting {
            exit.write(AppExit::Success);
        } else if desktop.bridge.take_reset() {
            let mut command = Command::new(&desktop.program);
            command.args(&desktop.args);
            match Source::spawn(command) {
                Ok(source) => {
                    desktop.source = source;
                    desktop.bridge.reset();
                    desktop.bridge.running(true);
                    desktop.failed = false;
                    desktop.stopping = false;
                }
                Err(error) => eprintln!("Could not restart workload: {error}"),
            }
        }
        return;
    }
    let result = (|| -> Result<bool, String> {
        if quitting {
            desktop.restarting = false;
            if !desktop.stopping {
                desktop.source.stop().map_err(|e| e.to_string())?;
                desktop.stopping = true;
            }
        } else if desktop.bridge.take_reset() && !desktop.stopping {
            desktop.source.stop().map_err(|e| e.to_string())?;
            desktop.stopping = true;
            desktop.restarting = true;
        }
        if !desktop.stopping {
            for (key, input) in [
                (KeyCode::KeyW, b'w'),
                (KeyCode::ArrowUp, b'w'),
                (KeyCode::KeyS, b's'),
                (KeyCode::ArrowDown, b's'),
                (KeyCode::KeyA, b'a'),
                (KeyCode::ArrowLeft, b'a'),
                (KeyCode::KeyD, b'd'),
                (KeyCode::ArrowRight, b'd'),
                (KeyCode::Space, b' '),
            ] {
                if keys.just_pressed(key) {
                    desktop.source.input(input).map_err(|e| e.to_string())?;
                }
            }
        }
        let bridge = desktop.bridge.clone();
        let done = desktop.source.poll(&bridge)?;
        if let Some(report) = bridge.snapshot().report {
            if let Ok(mut window) = windows.single_mut() {
                window.title =
                    if desktop.stopping { "ConTime / Finishing validation" } else { "ConTime / Reconciliation in motion" }.into();
            }
            if done {
                println!(
                    "Validated {} delivered inputs; {} reordered arrivals; {} corrected histories; {} keyboard inputs",
                    report.submitted, report.late, report.corrections, report.keyboard_inputs
                );
            }
        }
        if done && desktop.restarting {
            let mut command = Command::new(&desktop.program);
            command.args(&desktop.args);
            desktop.source = Source::spawn(command).map_err(|e| e.to_string())?;
            desktop.bridge.reset();
            desktop.bridge.running(true);
            desktop.stopping = false;
            desktop.restarting = false;
            return Ok(false);
        }
        Ok(done)
    })();
    match result {
        Ok(true) => {
            desktop.bridge.finish();
            exit.write(AppExit::Success);
        }
        Err(error) => {
            eprintln!("Example workload failed: {error}");
            if desktop.bridge.snapshot().dashboard.as_ref().is_some_and(|dashboard| dashboard.failure.is_some()) {
                desktop.failed = true;
                desktop.bridge.finish();
            } else {
                exit.write(AppExit::error());
            }
        }
        Ok(false) => {}
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let program = args.next().ok_or_else(|| io::Error::other("Pass a workload executable followed by its arguments"))?;
    let args: Vec<_> = args.collect();
    let mut command = Command::new(&program);
    command.args(&args);
    let source = Source::spawn(command)?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&interrupted);
    bevy::app::ctrlc::set_handler(move || signal.store(true, Ordering::Release))?;
    let bridge = Bridge::default();
    bridge.running(true);
    let mut app = scene::app(bridge.clone(), None);
    app.insert_non_send(Desktop { source, bridge, interrupted, stopping: false, failed: false, restarting: false, program, args })
        .add_systems(Update, desktop.in_set(GraphicsSet::Collect));
    if app.run().is_success() { Ok(()) } else { Err(io::Error::other("Graphical movement example failed").into()) }
}
