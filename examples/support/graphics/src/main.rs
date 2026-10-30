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
use std::{io, process::Command};

struct Desktop {
    source: Source,
    bridge: Bridge,
    stopping: bool,
}
fn desktop(
    mut desktop: NonSendMut<Desktop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut close: MessageReader<WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let result = (|| -> Result<bool, String> {
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
            if close.read().next().is_some() || keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyQ) {
                desktop.source.stop().map_err(|e| e.to_string())?;
                desktop.stopping = true;
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
        Ok(done)
    })();
    match result {
        Ok(true) => {
            desktop.bridge.finish();
            exit.write(AppExit::Success);
        }
        Err(error) => {
            eprintln!("Movement workload failed: {error}");
            exit.write(AppExit::error());
        }
        Ok(false) => {}
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let program = args.next().ok_or_else(|| io::Error::other("Pass a workload executable followed by its arguments"))?;
    let mut command = Command::new(program);
    command.args(args);
    let source = Source::spawn(command)?;
    let bridge = Bridge::default();
    bridge.running(true);
    let mut app = scene::app(bridge.clone(), None);
    app.insert_non_send(Desktop { source, bridge, stopping: false }).add_systems(Update, desktop.in_set(GraphicsSet::Collect));
    if app.run().is_success() { Ok(()) } else { Err(io::Error::other("Graphical movement example failed").into()) }
}
