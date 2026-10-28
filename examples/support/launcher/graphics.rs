//! Build the isolated Bevy presenter; the selected workload keeps its own process.
use super::{
    launch::{self, CommandSpec},
    selection::Selection,
};
use std::{path::Path, sync::atomic::AtomicBool};
type Failure = Box<dyn std::error::Error>;

pub(super) fn build(root: &Path, browser: bool) -> CommandSpec {
    let mut args = vec!["+1.95.0", "build"];
    if browser {
        args.extend(["--release", "--lib", "--target", "wasm32-unknown-unknown"]);
    }
    let directory = root.join("examples/support/graphics");
    args.push("--target-dir");
    let mut command = CommandSpec::new("cargo", &directory, args);
    command.args.push(directory.join("target").into_os_string());
    command
}
pub(super) fn bindings(root: &Path, program: std::ffi::OsString) -> CommandSpec {
    CommandSpec::new(
        program,
        &root.join("examples/support/graphics"),
        [
            "target/wasm32-unknown-unknown/release/contime_graphics_example.wasm",
            "--target",
            "web",
            "--out-dir",
            "../composition/browser/pkg/graphics",
        ],
    )
}
pub(super) fn present(root: &Path, guest: CommandSpec, dry: bool, cancelled: &AtomicBool) -> Result<(), Failure> {
    let build = build(root, false);
    let mut args = vec![guest.program];
    args.extend(guest.args);
    let presenter = CommandSpec::new(root.join("examples/support/graphics/target/debug/contime-graphics-example"), &guest.cwd, args);
    if dry {
        build.show();
        presenter.show();
        return Ok(());
    }
    println!("Building shared Bevy presentation (Rust 1.95)…");
    build.run()?;
    launch::native(presenter, false, cancelled)
}
pub(super) fn native(root: &Path, selection: Selection, arguments: &[String], dry: bool, cancelled: &AtomicBool) -> Result<(), Failure> {
    let cwd = root.join("examples/environments/wasi");
    // The portable workload is compiled to a native binary here, not WebAssembly.
    let mut build = CommandSpec::new("cargo", &cwd, ["+1.90.0", "build", "--release", "--features", "runtime", "--target-dir"]);
    build.args.push(cwd.join("target").into_os_string());
    let mut args = vec![
        "--domain".into(),
        "client-movement".into(),
        "--execution".into(),
        selection.execution.slug().into(),
        "--metrics-stdio".into(),
        "--control-input".into(),
        "poll".into(),
        "--no-setup".into(),
    ];
    args.extend_from_slice(arguments);
    let guest = CommandSpec::new(cwd.join("target/release/contime-wasm-example"), &cwd, args);
    if dry {
        build.show();
    } else {
        println!("Building native movement workload…");
        build.run()?;
    }
    present(root, guest, dry, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_is_isolated_from_the_portable_workload_toolchain() {
        let root = Path::new("/a checkout");
        let native = build(root, false);
        assert_eq!(native.cwd, root.join("examples/support/graphics"));
        assert_eq!(
            native.args,
            [
                std::ffi::OsString::from("+1.95.0"),
                "build".into(),
                "--target-dir".into(),
                root.join("examples/support/graphics/target").into_os_string()
            ]
        );
        let browser = build(root, true);
        assert!(browser.args.iter().any(|arg| arg == "wasm32-unknown-unknown"));
        assert!(browser.args.iter().any(|arg| arg == "--lib"));
    }
}
