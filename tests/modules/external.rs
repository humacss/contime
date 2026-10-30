//! Independently compiled providers exercise exported metadata and Cargo renames.
use std::{fs, path::PathBuf, process::Command};

#[test]
fn external_modules_keep_crate_identity_and_accept_renamed_contime() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("tests/modules/fixtures/external");
    let workspace = root.join("target/module-external-source");
    let root_toml = root.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    for package in ["one", "two", "consumer"] {
        let destination = workspace.join(package);
        fs::create_dir_all(destination.join("src")).expect("create fixture package");
        let manifest = fs::read_to_string(source.join(package).join("Cargo.toml.fixture")).expect("read packaged fixture manifest");
        fs::write(destination.join("Cargo.toml"), manifest.replace("@CONTIME_ROOT@", &root_toml)).expect("materialize fixture manifest");
        fs::copy(source.join(package).join("src/lib.rs"), destination.join("src/lib.rs")).expect("copy fixture source");
    }
    let output = Command::new(env!("CARGO"))
        .args(["test", "--offline", "--manifest-path"])
        .arg(workspace.join("consumer/Cargo.toml"))
        .arg("--target-dir")
        .arg(root.join("target/module-external-fixture"))
        .output()
        .expect("run external fixture Cargo");
    assert!(
        output.status.success(),
        "external fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
