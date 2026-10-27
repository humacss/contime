//! Process exit belongs only to standalone example executables.
use std::io::{self, Write};
/// Called only after final validation, terminal restoration and the summary.
pub fn success() -> io::Result<()> {
    // No active output locks or terminal guards may survive this call. This is
    // process-wide: worker joins and history destructors are intentionally skipped.
    io::stdout().flush()?;
    io::stderr().flush()?;
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, process::Command};
    struct RetainedHistory(PathBuf);
    impl Drop for RetainedHistory {
        fn drop(&mut self) {
            std::fs::write(&self.0, "dropped").unwrap();
        }
    }
    #[test]
    fn success_flushes_reports_and_skips_history_destructors() {
        const CHILD: &str = "CONTIME_EXAMPLE_EXIT_TEST";
        if let Some(marker) = std::env::var_os(CHILD) {
            let _history = RetainedHistory(marker.into());
            print!("validated summary");
            eprint!("final diagnostic");
            success().unwrap();
            return;
        }
        let marker = std::env::temp_dir().join(format!("contime-exit-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let module = module_path!().split_once("::").unwrap().1;
        let test_name = format!("{module}::success_flushes_reports_and_skips_history_destructors");
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD, &marker)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("validated summary"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("final diagnostic"));
        let dropped = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(!dropped, "successful process exit must leave retained histories to the OS");
    }
}
