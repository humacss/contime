//! Enable the launcher's dashboard without changing the library's defaults.
use std::{ffi::OsString, path::Path, process::Command};

fn command(root: &Path, release: bool, arguments: impl IntoIterator<Item = OsString>) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command.args(["run", "--manifest-path"]).arg(root.join("Cargo.toml"));
    command.args(["--all-features", "--example", "launcher"]);
    if release {
        command.arg("--release");
    }
    command.arg("--").args(arguments);
    command
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Cargo releases its build lock before running an example. The second build
    // enables Runtime and monitor, so it enters the dashboard instead of here.
    // Preserve the standard debug/release choice and forward arguments verbatim.
    let status = command(Path::new(env!("CARGO_MANIFEST_DIR")), !cfg!(debug_assertions), std::env::args_os().skip(1)).status()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_launch_enables_features_and_preserves_arguments() {
        let arguments = ["--environment", "native", "--execution", "manual", "--", "--headless"].map(OsString::from);
        let command = command(Path::new("/checkout with spaces"), false, arguments.clone());
        let args: Vec<_> = command.get_args().collect();
        assert!(args.contains(&std::ffi::OsStr::new("--all-features")));
        assert!(!args.contains(&std::ffi::OsStr::new("--release")));
        assert_eq!(args[2], Path::new("/checkout with spaces/Cargo.toml"));
        assert_eq!(&args[args.len() - arguments.len()..], arguments.as_slice());
    }

    #[test]
    fn release_launch_preserves_the_launcher_profile() {
        let command = command(Path::new("/checkout"), true, std::iter::empty());
        assert!(command.get_args().any(|arg| arg == "--release"));
    }
}
