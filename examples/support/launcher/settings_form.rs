//! Compose startup definitions and adapt selected values to existing case parsers.
//! No ConTime instance is created until this form has been validated.
use super::{
    form::{Field, Form},
    selection::{BrowserSettings, Domain, Environment, Execution, Options, Selection},
};
use crate::support::{
    movement_config::Config,
    schema::{self, Definition, ValueKind},
    Settings,
};
use std::io;

pub(crate) struct SettingsForm {
    definitions: Vec<Definition>,
    form: Form,
    selection: Selection,
    options: Options,
}
impl SettingsForm {
    pub fn new(selection: Selection, options: &Options) -> Result<Self, String> {
        options.validate(selection)?;
        let mut result = Self { definitions: Vec::new(), form: Form::new(Vec::new(), true), selection, options: options.clone() };
        result.definitions = if selection.domain == Domain::Movement {
            schema::movement(&result.parse_movement(&options.example_args)?)
        } else {
            schema::common(&result.parse_common(&options.example_args)?, selection.execution == Execution::Manual)
        };
        if selection.environment == Environment::Browser {
            let browser = options.browser();
            result.definitions.extend(schema::browser(browser.budget_ms, browser.max_steps));
        }
        result.form = Form::new(
            result
                .definitions
                .iter()
                .map(|definition| {
                    let mut field = Field::new(definition.label, definition.help, definition.default.clone(), definition.choices.clone());
                    field.decimal = matches!(definition.kind, ValueKind::Decimal { .. } | ValueKind::Average { .. });
                    field.group = Some(definition.group);
                    field.allowed = Some(format!("Allowed: {}.\n", definition.allowed()));
                    field
                })
                .collect(),
            true,
        );
        let mut form = std::mem::replace(&mut result.form, Form::new(Vec::new(), true));
        result.refresh(&mut form);
        result.form = form;
        Ok(result)
    }
    fn parse_common(&self, args: &[String]) -> Result<Settings, String> {
        let mut settings = Settings::parse(args.to_vec(), self.selection.execution == Execution::Manual)
            .map_err(|e| e.to_string())?
            .ok_or("Use launcher --help for usage")?
            .0;
        if self.selection.environment != Environment::Terminal && !args.iter().any(|arg| arg == "--duration") {
            settings.duration = None;
        }
        Ok(settings)
    }
    fn parse_movement(&self, args: &[String]) -> Result<Config, String> {
        Config::parse(args.iter().cloned().chain(["--execution".into(), self.selection.execution.slug().into()]))?
            .ok_or_else(|| "Use launcher --help for usage".into())
    }
    fn arguments_from(&self, form: &Form) -> Result<Vec<String>, String> {
        let mut args = self.options.example_args.clone();
        for (definition, field) in self.definitions.iter().zip(&form.fields) {
            if !field.visible {
                continue;
            }
            let value = definition.normalize(&field.value)?;
            if field.changed() && !matches!(definition.id, "--budget-ms" | "--max-steps") {
                args.extend([definition.id.into(), value]);
            }
        }
        if !args.iter().any(|arg| arg == "--no-setup") {
            args.push("--no-setup".into());
        }
        Ok(args)
    }
    pub fn arguments(&self) -> Result<Vec<String>, String> {
        self.arguments_from(&self.form)
    }
    pub fn common(&self) -> Result<Settings, String> {
        self.parse_common(&self.arguments()?)
    }
    pub fn movement(&self) -> Result<Config, String> {
        self.parse_movement(&self.arguments()?)
    }
    fn browser_from(&self, form: &Form) -> Result<BrowserSettings, String> {
        let mut options = self.options.clone();
        for (definition, field) in self.definitions.iter().zip(&form.fields).filter(|(d, _)| matches!(d.id, "--budget-ms" | "--max-steps"))
        {
            let value = definition.normalize(&field.value)?;
            match definition.id {
                "--budget-ms" => options.budget_ms = Some(value.parse().map_err(|_| "Invalid budget")?),
                "--max-steps" => options.max_steps = Some(value.parse().map_err(|_| "Invalid step count")?),
                _ => return Err("Unknown browser setting".into()),
            }
        }
        Ok(options.browser())
    }
    pub fn browser(&self) -> Result<BrowserSettings, String> {
        self.browser_from(&self.form)
    }
    fn validate(&self, form: &mut Form) -> Result<(), String> {
        // Focus the first invalid visible field and make the next keystroke replace it.
        for (index, (definition, field)) in self.definitions.iter().zip(&form.fields).enumerate() {
            if field.visible {
                if let Err(error) = definition.normalize(&field.value) {
                    form.focus(index);
                    return Err(error);
                }
            }
        }
        let result = self.validate_values(form);
        if let Err(error) = &result {
            if let Some(index) = self.definitions.iter().position(|definition| error.contains(definition.id)) {
                form.focus(index);
            }
        }
        result
    }
    fn validate_values(&self, form: &Form) -> Result<(), String> {
        let args = self.arguments_from(form)?;
        if self.selection.domain == Domain::Movement {
            self.parse_movement(&args)?;
        } else {
            self.parse_common(&args)?;
        }
        self.browser_from(form)?;
        Ok(())
    }
    fn refresh(&self, form: &mut Form) {
        // Visibility follows the value being edited, even while another field is invalid.
        let maximum = self
            .definitions
            .iter()
            .zip(&form.fields)
            .find(|(d, _)| d.id == "--events-per-second")
            .is_some_and(|(_, f)| matches!(f.value.as_str(), "Maximum" | "max"));
        for (definition, field) in self.definitions.iter().zip(&mut form.fields) {
            field.visible = definition.id != "--max-pending" || !maximum;
        }
        if !form.fields[form.selected].visible {
            form.selected = form.fields.iter().position(|f| f.visible).unwrap_or(0);
        }
        let Ok(args) = self.arguments_from(form) else {
            return;
        };
        let definitions = if self.selection.domain == Domain::Movement {
            self.parse_movement(&args).map(|c| schema::movement(&c))
        } else {
            self.parse_common(&args).map(|s| schema::common(&s, self.selection.execution == Execution::Manual))
        };
        // A temporary invalid value must not overwrite any other edit.
        if let Ok(definitions) = definitions {
            for definition in definitions {
                if let Some(index) = self.definitions.iter().position(|d| d.id == definition.id) {
                    form.fields[index].update_default(definition.default);
                }
            }
        }
    }
    pub fn run(&mut self) -> io::Result<bool> {
        self.run_named(self.selection.domain.label())
    }
    pub fn run_named(&mut self, title: &str) -> io::Result<bool> {
        let mut form = std::mem::replace(&mut self.form, Form::new(Vec::new(), true));
        let result = form.run(title, "start example", |form| self.refresh(form), |form| self.validate(form));
        self.form = form;
        result
    }
    #[cfg(test)]
    fn field(&self, id: &str) -> Option<&Field> {
        self.definitions.iter().position(|d| d.id == id).map(|i| &self.form.fields[i])
    }
    #[cfg(test)]
    fn edit(&mut self, id: &str, value: &str) {
        let index = self.definitions.iter().position(|d| d.id == id).unwrap();
        self.form.fields[index].value = value.into();
        let mut form = std::mem::replace(&mut self.form, Form::new(Vec::new(), true));
        self.refresh(&mut form);
        self.form = form;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session(environment: Environment, execution: Execution, domain: Domain, args: &[&str]) -> SettingsForm {
        let selection = Selection::new(environment, execution, domain).unwrap();
        let options = Options { example_args: args.iter().map(|s| s.to_string()).collect(), ..Options::default() };
        SettingsForm::new(selection, &options).unwrap()
    }
    #[test]
    fn groups_are_composed_for_the_selected_case_and_environment() {
        let movement = session(Environment::Terminal, Execution::Manual, Domain::Movement, &[]);
        assert!(movement.field("--average-latency-ms").is_some());
        assert!(movement.field("--events-per-second").is_none());
        assert!(movement.field("--workers").is_none());
        assert_eq!(movement.field("--clients").unwrap().group, Some("Client movement"));
        let browser = session(Environment::Browser, Execution::Manual, Domain::Counter, &[]);
        assert_eq!(browser.field("--budget-ms").unwrap().group, Some("Browser"));
        assert_eq!(browser.field("--retention-ms").unwrap().group, Some("ConTime"));
    }
    #[test]
    fn worker_edits_update_only_automatic_defaults() {
        let mut s = session(Environment::Terminal, Execution::Runtime, Domain::Counter, &[]);
        s.edit("--workers", "3");
        assert_eq!(s.field("--snapshots").unwrap().value, "3000");
        s.edit("--snapshots", "7");
        s.edit("--workers", "2");
        assert_eq!(s.common().unwrap().snapshot_count, 7);
        let mut explicit = session(Environment::Terminal, Execution::Runtime, Domain::Counter, &["--snapshots", "9"]);
        explicit.edit("--workers", "2");
        assert_eq!(explicit.common().unwrap().snapshot_count, 9);
    }
    #[test]
    fn movement_edits_validate_and_expand_automatic_retention() {
        let mut s = session(Environment::Terminal, Execution::Manual, Domain::Movement, &[]);
        s.edit("--average-latency-ms", "30000");
        let config = s.movement().unwrap();
        assert!(config.retention_ms > 32800);
        assert_eq!(s.field("--retention-ms").unwrap().value, config.retention_ms.to_string());
        s.edit("--retention-ms", "10000");
        assert!(s.movement().is_err());
        s.edit("--clients", "10001");
        assert!(s.arguments().is_err());
    }
    #[test]
    fn pending_is_hidden_only_in_maximum_mode_and_browser_values_are_validated() {
        let mut s = session(Environment::Browser, Execution::Manual, Domain::Hash, &[]);
        assert!(!s.field("--max-pending").unwrap().visible);
        s.edit("--events-per-second", "1000");
        assert!(s.field("--max-pending").unwrap().visible);
        assert_eq!(s.common().unwrap().event_rate.limit(), Some(1000));
        s.edit("--budget-ms", "0.5");
        assert_eq!(s.browser().unwrap().budget_ms, 0.5);
        for value in ["NaN", "0", "1001"] {
            s.edit("--budget-ms", value);
            assert!(s.browser().is_err());
        }
    }
    #[test]
    fn long_duration_and_explicit_retention_survive_round_trip() {
        let s = session(
            Environment::Terminal,
            Execution::Manual,
            Domain::Movement,
            &["--duration", "315360000", "--retention-ms", "60000", "--latency-ms", "1000"],
        );
        let c = s.movement().unwrap();
        assert_eq!(c.duration.unwrap().as_secs(), 315360000);
        assert_eq!(c.retention_ms, 60000);
        assert!(s.arguments().unwrap().contains(&"--no-setup".into()));
    }
    #[test]
    fn grouped_browser_form_keeps_ranges_and_selected_field_visible_at_80_by_24() {
        let mut s = session(Environment::Browser, Execution::Manual, Domain::Counter, &[]);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        for index in 0..s.form.fields.len() {
            if !s.form.fields[index].visible {
                continue;
            }
            s.form.selected = index;
            terminal.draw(|frame| s.form.render(frame, "Counter", "start example")).unwrap();
            let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains(s.form.fields[index].label), "{}", s.form.fields[index].label);
            assert!(text.contains("Allowed:") && text.contains("Enter start example") && text.contains("Esc cancel"));
        }
    }
    #[test]
    fn numeric_pending_edits_survive_maximum_and_default_changes() {
        let mut s = session(Environment::Terminal, Execution::Runtime, Domain::Counter, &[]);
        s.edit("--events-per-second", "1000");
        assert_eq!(s.field("--max-pending").unwrap().value, "4096");
        s.edit("--max-pending", "17");
        s.edit("--events-per-second", "Maximum");
        assert!(!s.field("--max-pending").unwrap().visible);
        s.edit("--events-per-second", "100000");
        assert_eq!(s.common().unwrap().max_pending, 17);
    }
    #[test]
    fn browser_and_case_changes_round_trip_without_forwarding_browser_flags_to_cases() {
        let mut s = session(Environment::Browser, Execution::Manual, Domain::Hash, &[]);
        s.edit("--snapshots", "8");
        s.edit("--average-snapshots-per-event", "1.5");
        s.edit("--max-snapshots-per-event", "3");
        s.edit("--max-steps", "100000");
        assert_eq!(s.common().unwrap().average_targets.to_string(), "1.5");
        assert_eq!(s.browser().unwrap().max_steps, 100000);
        assert!(!s.arguments().unwrap().iter().any(|a| a == "--max-steps"));
        s.edit("--max-steps", "100001");
        assert!(s.browser().is_err());
        s.edit("--max-steps", "1000");
        s.edit("--average-snapshots-per-event", "3.1");
        assert!(s.common().is_err());
    }
    #[test]
    fn configured_movement_values_are_available_and_forwarded_in_every_environment() {
        for environment in [Environment::Terminal, Environment::Wasm, Environment::WasmThreads, Environment::Browser] {
            let mut setup = session(environment, Execution::Manual, Domain::Movement, &[]);
            assert_eq!(setup.field("--client-spacing").unwrap().value, "16");
            assert_eq!(setup.field("--movement-speed").unwrap().value, "6");
            setup.edit("--client-spacing", "9");
            setup.edit("--movement-speed", "10");
            let config = setup.movement().unwrap();
            assert_eq!((config.client_spacing, config.movement_speed), (9, 10));
            let parsed = Config::parse(config.arguments()).unwrap().unwrap();
            assert_eq!((parsed.client_spacing, parsed.movement_speed), (9, 10));
            let url = setup.browser().unwrap().movement_url("http://localhost/", &config);
            assert!(url.contains("client_spacing=9") && url.contains("movement_speed=10"));
            setup.edit("--movement-speed", "1000001");
            assert!(setup.movement().is_err());
        }
    }
    fn press(s: &mut SettingsForm, id: &str, key: ratatui::crossterm::event::KeyCode) {
        let index = s.definitions.iter().position(|d| d.id == id).unwrap();
        if s.form.selected != index {
            s.form.focus(index);
        }
        s.form.handle(ratatui::crossterm::event::KeyEvent::new(key, ratatui::crossterm::event::KeyModifiers::NONE));
        let mut form = std::mem::replace(&mut s.form, Form::new(Vec::new(), true));
        s.refresh(&mut form);
        s.form = form;
    }
    fn validate(s: &mut SettingsForm) -> Result<(), String> {
        let mut form = std::mem::replace(&mut s.form, Form::new(Vec::new(), true));
        let result = s.validate(&mut form);
        s.form = form;
        result
    }
    #[test]
    fn direct_startup_arguments_preserve_effective_values_and_automatic_provenance() {
        for execution in [Execution::Manual, Execution::Runtime] {
            let args = [
                "--snapshots",
                "7",
                "--events-per-second",
                "100",
                "--max-pending",
                "17",
                "--average-snapshots-per-event",
                "1.375",
                "--max-snapshots-per-event",
                "3",
                "--snapshot-skew",
                "90",
                "--query-hz",
                "60",
                "--retention-ms",
                "60000",
                "--checkpoint-interval",
                "32",
                "--duration",
                "315360000",
            ];
            let form = session(Environment::Terminal, execution, Domain::Counter, &args);
            let expected = Settings::parse(args.map(str::to_owned), execution == Execution::Manual).unwrap().unwrap().0;
            assert_eq!(format!("{:?}", form.common().unwrap()), format!("{expected:?}"));
            assert!(form.field("--workers").is_some() == (execution == Execution::Runtime));
        }
        let mut s = session(
            Environment::Terminal,
            Execution::Runtime,
            Domain::Hash,
            &["--workers", "2", "--snapshots", "2000", "--events-per-second", "20000", "--max-pending", "40000"],
        );
        s.edit("--workers", "4");
        s.edit("--events-per-second", "100");
        let chosen = s.common().unwrap();
        assert_eq!((chosen.snapshot_count, chosen.max_pending), (2000, 40000));
        assert!(!chosen.automatic_snapshots && !chosen.automatic_pending);
    }
    #[test]
    fn keyboard_edits_keep_automatic_defaults_until_a_user_selects_the_current_value() {
        use ratatui::crossterm::event::KeyCode::*;
        let mut s = session(Environment::Terminal, Execution::Runtime, Domain::Counter, &["--workers", "2"]);
        press(&mut s, "--workers", Right);
        assert_eq!(s.field("--snapshots").unwrap().value, "4000");
        press(&mut s, "--events-per-second", Char('1'));
        assert_eq!(s.field("--max-pending").unwrap().value, "4096");
        for c in "4000".chars() {
            press(&mut s, "--snapshots", Char(c));
        }
        for c in "4096".chars() {
            press(&mut s, "--max-pending", Char(c));
        }
        press(&mut s, "--workers", Right);
        s.edit("--events-per-second", "100000");
        let chosen = s.common().unwrap();
        assert_eq!((chosen.snapshot_count, chosen.max_pending), (4000, 4096));
        assert!(!chosen.automatic_snapshots && !chosen.automatic_pending);
    }
    #[test]
    fn invalid_pending_is_retained_and_ignored_only_in_maximum_mode() {
        for execution in [Execution::Manual, Execution::Runtime] {
            for value in ["", "0", "10000001"] {
                let mut s = session(Environment::Terminal, execution, Domain::Hash, &["--events-per-second", "100"]);
                s.edit("--max-pending", value);
                s.edit("--events-per-second", "Maximum");
                assert!(validate(&mut s).is_ok());
                assert!(!s.field("--max-pending").unwrap().visible);
                s.edit("--events-per-second", "1");
                assert!(validate(&mut s).is_err());
                assert_eq!(s.definitions[s.form.selected].id, "--max-pending");
                assert_eq!(s.field("--max-pending").unwrap().value, value);
            }
        }
    }
    #[test]
    fn invalid_and_cross_field_values_focus_the_problem_and_replace_previous_input() {
        use ratatui::crossterm::event::KeyCode::*;
        for (id, value) in [
            ("--query-hz", "0"),
            ("--snapshots", "1000001"),
            ("--average-snapshots-per-event", "1.0001"),
            ("--average-snapshots-per-event", "2.1"),
            ("--max-snapshots-per-event", "5"),
        ] {
            let mut s = session(Environment::Terminal, Execution::Manual, Domain::Counter, &["--snapshots", "4"]);
            s.edit(id, value);
            assert!(validate(&mut s).is_err(), "{id}={value}");
            assert_eq!(s.definitions[s.form.selected].id, id);
            press(&mut s, id, Char('1'));
            assert_eq!(s.field(id).unwrap().value, "1");
            assert!(validate(&mut s).is_ok());
        }
    }
    #[test]
    fn decimal_average_edits_exactly_resets_and_ignores_decimal_on_integers() {
        use ratatui::crossterm::event::KeyCode::*;
        let mut s = session(Environment::Terminal, Execution::Manual, Domain::Counter, &[]);
        for c in "1.375".chars() {
            press(&mut s, "--average-snapshots-per-event", Char(c));
        }
        assert_eq!(s.common().unwrap().average_targets.scaled(), 1375);
        press(&mut s, "--average-snapshots-per-event", Backspace);
        assert_eq!(s.common().unwrap().average_targets.scaled(), 1370);
        press(&mut s, "--average-snapshots-per-event", Char('r'));
        assert_eq!(s.field("--average-snapshots-per-event").unwrap().value, "1.0");
        for expected in ["1.1", "1.5", "2.0"] {
            press(&mut s, "--average-snapshots-per-event", Right);
            assert_eq!(s.field("--average-snapshots-per-event").unwrap().value, expected);
        }
        press(&mut s, "--max-snapshots-per-event", Char('.'));
        assert_eq!(s.field("--max-snapshots-per-event").unwrap().value, "2");
    }
    #[test]
    fn every_visible_setting_renders_help_ranges_formatted_values_and_presets() {
        for width in [80, 120] {
            let mut s = session(Environment::Terminal, Execution::Runtime, Domain::Hash, &["--events-per-second", "100", "--workers", "3"]);
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
            for i in 0..s.form.fields.len() {
                s.form.selected = i;
                terminal.draw(|frame| s.form.render(frame, "Hash", "start example")).unwrap();
                let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains(s.form.fields[i].label));
                for expected in ["Allowed:", "Common values:", "Enter start example", "Esc cancel"] {
                    assert!(text.contains(expected), "{width}: {expected}");
                }
            }
            s.form.selected = s.definitions.iter().position(|d| d.id == "--workers").unwrap();
            terminal.draw(|frame| s.form.render(frame, "Hash", "start example")).unwrap();
            let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("1,024") && text.contains("worker threads") && text.contains("– / 11"));
        }
    }
    #[test]
    fn maximum_rate_presets_and_navigation_skip_hidden_pending_without_losing_focus() {
        use ratatui::crossterm::event::KeyCode::*;
        for execution in [Execution::Manual, Execution::Runtime] {
            let mut s =
                session(Environment::Terminal, execution, Domain::Counter, &["--events-per-second", "100", "--max-pending", "12345"]);
            for _ in 0..2 {
                press(&mut s, "--events-per-second", Char('m'));
                assert_eq!(s.definitions[s.form.selected].id, "--events-per-second");
                assert_eq!(s.field("--events-per-second").unwrap().value, "Maximum");
                assert_eq!(s.common().unwrap().event_rate, crate::support::EventRate::Maximum);
                let start = s.form.selected;
                let visible = s.form.fields.iter().filter(|f| f.visible).count();
                for _ in 0..visible {
                    s.form.handle(ratatui::crossterm::event::KeyEvent::new(Tab, ratatui::crossterm::event::KeyModifiers::NONE));
                    assert!(s.form.fields[s.form.selected].visible);
                }
                assert_eq!(s.form.selected, start);
                press(&mut s, "--events-per-second", Left);
                assert_eq!(s.field("--events-per-second").unwrap().value, "1000000000");
                assert_eq!(s.common().unwrap().max_pending, 12345);
            }
        }
    }
}
