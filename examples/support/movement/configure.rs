//! Terminal-only adapter for the shared movement configuration.
use super::movement_config::Config;
use std::io::IsTerminal;
impl Config {
    pub fn configure(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
        if self.headless || self.no_setup {
            return Ok(true);
        }
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Err("Use --headless --duration SECONDS without a terminal".into());
        }
        let selection = crate::support::launcher::selection::Selection::new(
            crate::support::launcher::selection::Environment::Terminal,
            if self.manual {
                crate::support::launcher::selection::Execution::Manual
            } else {
                crate::support::launcher::selection::Execution::Runtime
            },
            crate::support::launcher::selection::Domain::Movement,
        )?;
        let options = crate::support::launcher::selection::Options { example_args: self.arguments(), ..Default::default() };
        let mut setup = crate::support::launcher::settings_form::SettingsForm::new(selection, &options)?;
        if !setup.run()? {
            return Ok(false);
        }
        *self = setup.movement()?;
        Ok(true)
    }
}
