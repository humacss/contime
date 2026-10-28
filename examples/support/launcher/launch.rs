//! Dispatch separate examples. No application model or ConTime backend lives here.
use super::{
    selection::{BrowserSettings, Domain, Environment, Execution, Selection, WasmTarget},
    server::Server,
};
use crate::support::Settings;
use std::{
    ffi::OsString,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

const TOOLCHAIN: &str = "1.90.0";
#[derive(Debug)]
pub struct CommandSpec {
    pub(super) program: OsString,
    pub(super) args: Vec<OsString>,
    pub(super) cwd: PathBuf,
}
impl CommandSpec {
    pub(super) fn new(program: impl Into<OsString>, cwd: &Path, args: impl IntoIterator<Item = impl Into<OsString>>) -> Self {
        Self { program: program.into(), args: args.into_iter().map(Into::into).collect(), cwd: cwd.to_path_buf() }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).current_dir(&self.cwd);
        command
    }
    pub fn show(&self) {
        println!("  {:?}", self.command());
    }
    pub(super) fn run(&self) -> Result<(), Box<dyn std::error::Error>> {
        let status = self.command().status().map_err(|e| format!("Could not start {:?}: {e}", self.program))?;
        if !status.success() {
            return Err(format!("{:?} exited with {status}", self.program).into());
        }
        Ok(())
    }
}
/// Movement has client/network settings instead of throughput/fanout settings.
pub fn movement_command(root: &Path, selection: Selection, settings: &[String]) -> CommandSpec {
    let mut args = vec![
        "run".to_string(),
        "--quiet".into(),
        "--release".into(),
        "--manifest-path".into(),
        root.join("Cargo.toml").display().to_string(),
        "--features".into(),
        "runtime,manual,monitor,counters".into(),
        "--example".into(),
        "client_movement".into(),
        "--".into(),
        "--execution".into(),
        selection.execution.slug().into(),
    ];
    if !settings.iter().any(|arg| arg == "--no-setup") {
        args.push("--no-setup".into());
    }
    args.extend_from_slice(settings);
    CommandSpec::new("cargo", root, args)
}
pub fn native_command(root: &Path, selection: Selection, settings: &Settings) -> CommandSpec {
    let mut args = vec![
        "run".to_string(),
        "--quiet".into(),
        "--release".into(),
        "--manifest-path".into(),
        root.join("Cargo.toml").display().to_string(),
        "--features".into(),
        "runtime,manual,monitor,counters".into(),
        "--example".into(),
        selection.domain.binary().into(),
        "--".into(),
        "--execution".into(),
        selection.execution.slug().into(),
        "--no-setup".into(),
    ];
    args.extend(settings_arguments(settings, true));
    CommandSpec::new("cargo", root, args)
}
/// Serialize the common form once for both native and WASI child hosts.
fn settings_arguments(settings: &Settings, include_worker: bool) -> Vec<String> {
    let mut args = Vec::new();
    for (flag, value) in [
        ("--snapshots", settings.snapshot_count.to_string()),
        ("--snapshot-skew", settings.snapshot_skew.to_string()),
        ("--average-snapshots-per-event", settings.average_targets.to_string()),
        ("--max-snapshots-per-event", settings.max_targets.to_string()),
        ("--events-per-second", settings.event_rate.limit().map_or_else(|| "max".into(), |n| n.to_string())),
        ("--query-hz", settings.query_hz.to_string()),
        ("--retention-ms", settings.retention_ms.to_string()),
        ("--checkpoint-interval", settings.checkpoint_interval.to_string()),
    ] {
        args.extend([flag.into(), value]);
    }
    if settings.event_rate.limit().is_some() {
        args.extend(["--max-pending".into(), settings.max_pending.to_string()]);
    }
    if let Some(duration) = settings.duration {
        args.extend(["--duration".into(), duration.as_secs().to_string()]);
    }
    if settings.headless {
        args.push("--headless".into());
    }
    if include_worker {
        args.extend(["--workers".into(), settings.workers.to_string()]);
    }
    args
}
pub fn portable_build(root: &Path, environment: Environment) -> CommandSpec {
    let (directory, target) = match environment {
        Environment::Wasm => ("wasi", "wasm32-wasip1"),
        Environment::WasmThreads => ("wasi", "wasm32-wasip1-threads"),
        Environment::Browser => ("browser", "wasm32-unknown-unknown"),
        Environment::Terminal => unreachable!(),
    };
    let cwd = root.join("examples/environments").join(directory);
    CommandSpec::new(
        "cargo",
        &cwd,
        vec![
            format!("+{TOOLCHAIN}"),
            "build".into(),
            "--release".into(),
            "--target".into(),
            target.into(),
            "--target-dir".into(),
            cwd.join("target").display().to_string(),
        ],
    )
}
fn require_target(target: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed", "--toolchain", TOOLCHAIN])
        .output()
        .map_err(|_| format!("Install Rust {TOOLCHAIN} and target {target} with rustup before launching this environment"))?;
    if !output.status.success() || !String::from_utf8_lossy(&output.stdout).lines().any(|line| line == target) {
        return Err(format!("Missing Rust {TOOLCHAIN} target {target}. Run: rustup target add {target} --toolchain {TOOLCHAIN}").into());
    }
    Ok(())
}
fn resolve_tool_path(program: OsString, cwd: &Path) -> io::Result<OsString> {
    if Path::new(&program).components().count() > 1 {
        Ok(cwd.join(&program).canonicalize()?.into_os_string())
    } else {
        Ok(program)
    }
}
fn bindgen() -> Result<OsString, Box<dyn std::error::Error>> {
    let program = std::env::var_os("WASM_BINDGEN").unwrap_or_else(|| "wasm-bindgen".into());
    let program = resolve_tool_path(program, &std::env::current_dir()?)?;
    let output = Command::new(&program)
        .arg("--version")
        .output()
        .map_err(|error| format!("Could not run wasm-bindgen: {error}. Install wasm-bindgen-cli or set WASM_BINDGEN to its executable"))?;
    if !output.status.success() {
        return Err("Could not run wasm-bindgen --version; check the executable selected by PATH or WASM_BINDGEN".into());
    }
    Ok(program)
}
fn bindings_command(root: &Path, program: OsString) -> CommandSpec {
    let cwd = root.join("examples/environments/browser");
    CommandSpec::new(
        program,
        &cwd,
        [
            "target/wasm32-unknown-unknown/release/contime_browser_example.wasm",
            "--target",
            "web",
            "--out-dir",
            "../../support/composition/browser/pkg",
        ],
    )
}
pub fn native(spec: CommandSpec, dry: bool, cancelled: &AtomicBool) -> Result<(), Box<dyn std::error::Error>> {
    if dry {
        spec.show();
        return Ok(());
    }
    println!("Building and running the selected native example…");
    let result = spec.run();
    if cancelled.load(Ordering::Acquire) {
        Ok(())
    } else {
        result
    }
}
fn validate_wasmtime(version: &str) -> Result<(), String> {
    if version.split_whitespace().nth(1).is_some_and(|version| version.starts_with("36.")) {
        Ok(())
    } else {
        Err("This threaded WASI example requires Wasmtime 36.x LTS (tested with 36.0.19). Newer Wasmtime releases removed WASI threads. Set WASMTIME to a compatible executable; no tools were installed automatically".into())
    }
}
pub fn wasi_build(root: &Path, selection: Selection) -> CommandSpec {
    let mut build = portable_build(root, selection.environment);
    if selection.execution == Execution::Runtime {
        build.args.extend(["--features".into(), "runtime".into()]);
    }
    build
}
pub fn wasi_target_command(root: &Path, selection: Selection, settings: &Settings) -> CommandSpec {
    let target = selection.environment.wasm_target().unwrap_or(WasmTarget::Wasi);
    let mut run = if target == WasmTarget::Wasi {
        CommandSpec::new("node", &root.join("examples/environments/wasi"), ["run.mjs"])
    } else {
        CommandSpec::new(
            std::env::var_os("WASMTIME").unwrap_or_else(|| "wasmtime".into()),
            &root.join("examples/environments/wasi"),
            vec![
                "run".to_string(),
                "--wasi".into(),
                "threads=y".into(),
                format!("target/{}/release/contime-wasm-example.wasm", target.triple()),
            ],
        )
    };
    run.args.extend(["--domain".into(), selection.domain.slug().into(), "--execution".into(), selection.execution.slug().into()]);
    run.args.extend(settings_arguments(settings, selection.execution == Execution::Runtime).into_iter().map(OsString::from));
    run
}
fn wasi_case_command(root: &Path, selection: Selection, settings: &Settings, arguments: Option<&[String]>) -> CommandSpec {
    let mut run = wasi_target_command(root, selection, settings);
    if let Some(arguments) = arguments {
        let index = run.args.iter().position(|a| a == "--execution").expect("execution argument") + 2;
        run.args.truncate(index);
        run.args.extend(arguments.iter().map(OsString::from));
    }
    run
}

pub fn wasi(
    root: &Path,
    selection: Selection,
    settings: &Settings,
    case_arguments: Option<&[String]>,
    dry: bool,
    cancelled: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let target = selection.environment.wasm_target().unwrap_or(WasmTarget::Wasi);
    let build = wasi_build(root, selection);
    let mut run = wasi_case_command(root, selection, settings, case_arguments);
    if dry {
        build.show();
        if selection.domain == Domain::Movement && !settings.headless {
            run.args.extend([
                "--metrics-stdio".into(),
                "--control-input".into(),
                if target == WasmTarget::Threads { "poll".into() } else { "nonblocking".into() },
            ]);
            return super::graphics::present(root, run, true, cancelled);
        }
        run.show();
        return Ok(());
    }
    require_target(target.triple())?;
    run.program = resolve_tool_path(run.program, &std::env::current_dir()?)?;
    let status = Command::new(&run.program).arg("--version").output().map_err(|_| match target {
        WasmTarget::Wasi => "Standalone WASI needs Node.js with WASI support",
        WasmTarget::Threads => "Threaded WASI needs Wasmtime 36.x LTS; set WASMTIME to its executable",
    })?;
    if !status.status.success() {
        return Err("Could not run the selected WebAssembly host".into());
    }
    if target == WasmTarget::Threads {
        validate_wasmtime(&String::from_utf8_lossy(&status.stdout))?;
    }
    println!("Building standalone WebAssembly ({})…", target.triple());
    let result = (|| {
        build.run()?;
        println!("Running {} execution…", selection.execution.slug());
        if selection.domain == Domain::Movement && !settings.headless {
            run.args.extend([
                "--metrics-stdio".into(),
                "--control-input".into(),
                if target == WasmTarget::Threads { "poll".into() } else { "nonblocking".into() },
            ]);
            super::graphics::present(root, run, false, cancelled)
        } else if super::terminal::enabled(settings) {
            run.args.push("--metrics-stdio".into());
            if selection.domain == Domain::Movement {
                run.args
                    .extend(["--control-input".into(), if target == WasmTarget::Threads { "poll".into() } else { "nonblocking".into() }]);
            }
            super::terminal::run(run.command(), cancelled, selection.domain == Domain::Movement)
        } else {
            run.run()
        }
    })();
    if cancelled.load(Ordering::Acquire) {
        Ok(())
    } else {
        result
    }
}

#[allow(clippy::too_many_arguments)]
pub fn browser(
    root: &Path,
    domain: Domain,
    common: &Settings,
    movement: Option<&crate::support::movement_config::Config>,
    settings: BrowserSettings,
    dry: bool,
    no_open: bool,
    cancelled: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let build = portable_build(root, Environment::Browser);
    if dry {
        build.show();
        let generator = std::env::var_os("WASM_BINDGEN").unwrap_or_else(|| "wasm-bindgen".into());
        bindings_command(root, generator.clone()).show();
        if domain == Domain::Movement {
            super::graphics::build(root, true).show();
            super::graphics::bindings(root, generator).show();
        }
        println!("  Serve examples/support/composition/browser on 127.0.0.1 with an available port");
        println!(
            "  {} {}",
            if no_open { "URL:" } else { "Open:" },
            movement.map_or_else(
                || settings.url("http://127.0.0.1:<assigned-port>/", domain, common),
                |config| settings.movement_url("http://127.0.0.1:<assigned-port>/", config)
            )
        );
        return Ok(());
    }
    require_target("wasm32-unknown-unknown")?;
    let generator = bindgen()?;
    println!("Building browser WebAssembly…");
    build.run()?;
    println!("Generating browser bindings…");
    bindings_command(root, generator.clone()).run()?;
    if domain == Domain::Movement {
        println!("Building shared Bevy browser presentation (Rust 1.95)…");
        super::graphics::build(root, true).run()?;
        super::graphics::bindings(root, generator).run()?;
    }
    let server = Server::start(&root.join("examples/support/composition/browser"))?;
    let url = movement.map_or_else(|| settings.url(&server.url(), domain, common), |config| settings.movement_url(&server.url(), config));
    println!("\nBrowser example: {url}\nSettings are fixed for this run. Press Ctrl-C here to stop the server.");
    io::stdout().flush()?;
    if !no_open {
        if let Err(error) = open_browser(&url) {
            eprintln!("Could not open a browser automatically: {error}. Open the URL above manually.");
        }
    }
    while !cancelled.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(50));
    }
    drop(server);
    println!("Browser server stopped.");
    Ok(())
}
fn open_browser(url: &str) -> io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        Command::new("xdg-open")
    };
    let status = command.arg(url).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("browser opener exited with {status}")))
    }
}
#[cfg(test)]
mod tests {
    use super::super::selection::Execution;
    use super::*;
    #[test]
    fn pending_capacity_is_forwarded_only_for_numeric_rates_to_every_child_host() {
        for rate in ["max", "100"] {
            let settings =
                Settings::parse(["--events-per-second", rate, "--max-pending", "64"].map(str::to_owned), true).unwrap().unwrap().0;
            for environment in [Environment::Terminal, Environment::Wasm, Environment::WasmThreads] {
                let selection = Selection::new(environment, Execution::Manual, Domain::Counter).unwrap();
                let command = if environment == Environment::Terminal {
                    native_command(Path::new("/checkout"), selection, &settings)
                } else {
                    wasi_target_command(Path::new("/checkout"), selection, &settings)
                };
                assert_eq!(command.args.contains(&"--max-pending".into()), rate != "max");
                if rate != "max" {
                    assert!(command.args.windows(2).any(|pair| pair == [OsString::from("--max-pending"), OsString::from("64")]));
                }
            }
        }
    }
    #[test]
    fn native_settings_are_forwarded_exactly_to_separate_example_without_a_second_form() {
        let (settings, _) = Settings::parse(
            [
                "--average-snapshots-per-event",
                "1.1",
                "--max-snapshots-per-event",
                "4",
                "--snapshot-skew",
                "90",
                "--headless",
                "--duration",
                "1",
            ]
            .map(str::to_owned),
            true,
        )
        .unwrap()
        .unwrap();
        let selection = Selection::new(Environment::Terminal, Execution::Manual, Domain::Counter).unwrap();
        let command = native_command(Path::new("/a checkout with spaces"), selection, &settings);
        let args = command.args.iter().map(|s| s.to_string_lossy().to_string()).collect::<Vec<_>>();
        assert_eq!(command.cwd, Path::new("/a checkout with spaces"));
        assert!(args.windows(2).any(|pair| pair == ["--example", "counter_live"]));
        assert!(args.windows(2).any(|pair| pair == ["--average-snapshots-per-event", "1.1"]));
        assert!(args.windows(2).any(|pair| pair == ["--max-snapshots-per-event", "4"]));
        assert!(args.windows(2).any(|pair| pair == ["--execution", "manual"]));
        assert!(args.windows(2).any(|pair| pair == ["--snapshot-skew", "90"]));
        assert_eq!(args.iter().filter(|arg| *arg == "--no-setup").count(), 1);
        assert!(args.contains(&"--headless".into()));
    }
    #[test]
    fn portable_builds_use_the_host_directory_and_runner_output_location() {
        let root = Path::new("/checkout");
        let browser = portable_build(root, Environment::Browser);
        assert_eq!(browser.cwd, root.join("examples/environments/browser"));
        assert!(browser.args.contains(&OsString::from("wasm32-unknown-unknown")));
        let wasi = portable_build(root, Environment::Wasm);
        assert_eq!(wasi.cwd, root.join("examples/environments/wasi"));
        assert!(wasi.args.contains(&OsString::from("/checkout/examples/environments/wasi/target")));
        let settings = Settings::parse(Vec::new(), true).unwrap().unwrap().0;
        let selection = Selection::new(Environment::Wasm, Execution::Manual, Domain::Hash).unwrap();
        let run = wasi_target_command(root, selection, &settings);
        assert!(run.args.windows(2).any(|pair| pair == [OsString::from("--domain"), OsString::from("chain-hash")]));
    }
    #[test]
    fn wasi_forwards_the_same_live_settings_for_both_targets_and_stops_only_explicitly() {
        for environment in [Environment::Wasm, Environment::WasmThreads] {
            let selection = Selection::new(environment, Execution::Manual, Domain::Counter).unwrap();
            let settings = Settings::parse(
                [
                    "--snapshots",
                    "17",
                    "--events-per-second",
                    "400",
                    "--query-hz",
                    "7",
                    "--average-snapshots-per-event",
                    "1.1",
                    "--max-snapshots-per-event",
                    "4",
                ]
                .map(str::to_owned),
                true,
            )
            .unwrap()
            .unwrap()
            .0;
            let run = wasi_target_command(Path::new("/checkout"), selection, &settings);
            let args: Vec<_> = run.args.iter().map(|arg| arg.to_string_lossy().to_string()).collect();
            for (flag, value) in [
                ("--snapshots", "17"),
                ("--events-per-second", "400"),
                ("--query-hz", "7"),
                ("--average-snapshots-per-event", "1.1"),
                ("--max-snapshots-per-event", "4"),
            ] {
                assert!(args.windows(2).any(|pair| pair == [flag, value]));
            }
            assert!(!args.iter().any(|arg| matches!(arg.as_str(), "--rounds" | "--events-per-round" | "--duration")));
            let mut finite = settings;
            finite.duration = Some(Duration::from_secs(2));
            let run = wasi_target_command(Path::new("/checkout"), selection, &finite);
            assert!(run.args.windows(2).any(|pair| pair == [OsString::from("--duration"), OsString::from("2")]));
        }
    }
    #[test]
    fn relative_tool_paths_are_resolved_before_changing_host_directories() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let resolved = resolve_tool_path(OsString::from("./Cargo.toml"), root).unwrap();
        assert_eq!(PathBuf::from(resolved), root.join("Cargo.toml").canonicalize().unwrap());
        assert_eq!(resolve_tool_path(OsString::from("wasm-bindgen"), root).unwrap(), OsString::from("wasm-bindgen"));
    }
}

#[cfg(test)]
mod threaded_tests {
    use super::*;
    #[test]
    fn incompatible_wasmtime_fails_before_building() {
        assert!(validate_wasmtime("wasmtime 36.0.19 (hash)").is_ok());
        assert!(validate_wasmtime("wasmtime 50.0.0 (hash)").unwrap_err().contains("removed WASI threads"));
        assert!(validate_wasmtime("").is_err());
    }
    #[test]
    fn threaded_target_build_and_host_follow_execution_selection() {
        let root = Path::new("/checkout");
        for execution in [Execution::Manual, Execution::Runtime] {
            let selection = Selection::new(Environment::WasmThreads, execution, Domain::Hash).unwrap();
            let build = wasi_build(root, selection);
            assert!(build.args.contains(&"wasm32-wasip1-threads".into()));
            assert_eq!(build.args.contains(&"--features".into()), execution == Execution::Runtime);
            let settings = Settings::parse(
                ["--workers", if execution == Execution::Runtime { "4" } else { "1" }].map(str::to_owned),
                execution == Execution::Manual,
            )
            .unwrap()
            .unwrap()
            .0;
            let run = wasi_target_command(root, selection, &settings);
            assert!(run.args.contains(&"threads=y".into()));
            assert!(run.args.iter().any(|arg| arg.to_string_lossy().contains("wasm32-wasip1-threads/release/")));
            assert_eq!(run.args.contains(&"--workers".into()), execution == Execution::Runtime);
            assert!(run.args.windows(2).any(|pair| pair == [OsString::from("--execution"), OsString::from(execution.slug())]));
        }
    }
}

#[cfg(test)]
mod monitor_features_tests {
    use super::*;
    #[test]
    fn native_counter_and_hash_builds_enable_monitor_counters_for_both_execution_choices() {
        for domain in [Domain::Counter, Domain::Hash] {
            for execution in [Execution::Runtime, Execution::Manual] {
                let selection = Selection::new(Environment::Terminal, execution, domain).unwrap();
                let command =
                    native_command(Path::new("/checkout"), selection, &Settings::parse(std::iter::empty(), false).unwrap().unwrap().0);
                let features = command.args.windows(2).find(|pair| pair[0] == "--features").unwrap();
                assert_eq!(features[1], "runtime,manual,monitor,counters");
            }
        }
    }
}
