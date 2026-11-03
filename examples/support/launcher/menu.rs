//! Launcher forms. All hosts reuse the existing startup form.
use super::{
    form::{Field, Form},
    selection::{Domain, Environment, Execution, Options, Selection},
};

const ENVIRONMENTS: [&str; 4] = ["Native", "WebAssembly / WASI", "WebAssembly / WASI Threads", "Browser (WebAssembly)"];
const EXECUTIONS: [&str; 2] = ["runtime", "manual"];
const EXAMPLES: [&str; 3] = ["Counter", "Chain hash verification", "Client movement"];
fn field(label: &'static str, help: &'static str, value: impl ToString, choices: &[&str]) -> Field {
    Field::new(label, help, value.to_string(), choices.iter().map(|s| s.to_string()).collect())
}
fn selection_form(options: &Options) -> Result<Form, String> {
    // Invalid explicit command-line combinations remain errors, rather than
    // silently changing an explicitly requested backend.
    let selected = options.defaults()?;
    let mut form = Form::new(vec![
        field("Environment", "Native runs a local executable. Client movement opens a Bevy graphics window; other cases use the terminal. WASI uses Node (Manual only). WASI Threads uses Wasmtime 36.x LTS (Manual or Runtime). Browser opens a local WebAssembly page (Manual only).", selected.environment.label(), &ENVIRONMENTS),
        field("Execution", "Runtime uses background worker threads. Manual runs one worker on the calling thread. Runtime is available for Terminal or WASI threads. Standard WASI and Browser support Manual only.", selected.execution.slug(), &EXECUTIONS),
        field("Example", "Counter shows changing totals. Chain hash verifies event ordering, including late events. Client movement shows delayed client inputs and corrected paths.", selected.domain.label(), &EXAMPLES),
    ], false);
    if options.execution.is_some() {
        form.fields[1].remember_choice();
    }
    if options.domain.is_some() {
        form.fields[2].remember_choice();
    }
    constrain(&mut form);
    Ok(form)
}
fn constrain(form: &mut Form) {
    let runtime = form.fields[0].value == ENVIRONMENTS[0] || form.fields[0].value == ENVIRONMENTS[2];
    form.fields[1].set_choices(if runtime { EXECUTIONS.iter().map(|s| s.to_string()).collect() } else { vec!["manual".into()] });
    form.fields[2].set_choices(EXAMPLES.iter().map(|s| s.to_string()).collect());
}
fn selected(form: &Form) -> Result<Selection, String> {
    let environment = match form.fields[0].value.as_str() {
        value if value == ENVIRONMENTS[0] => Environment::Terminal,
        value if value == ENVIRONMENTS[1] => Environment::Wasm,
        value if value == ENVIRONMENTS[2] => Environment::WasmThreads,
        _ => Environment::Browser,
    };
    let execution = if form.fields[1].value == "runtime" { Execution::Runtime } else { Execution::Manual };
    let domain = match form.fields[2].value.as_str() {
        "Counter" => Domain::Counter,
        "Client movement" => Domain::Movement,
        _ => Domain::Hash,
    };
    Selection::new(environment, execution, domain)
}

pub fn selection(options: &Options, interactive: bool) -> Result<Option<Selection>, Box<dyn std::error::Error>> {
    if !interactive {
        return options.defaults().map(Some).map_err(Into::into);
    }
    let mut form = selection_form(options)?;
    if !form.run("Example launcher", "continue to settings", constrain, |form| options.validate(selected(form)?))? {
        return Ok(None);
    }
    Ok(Some(selected(&form)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_selection_limits_execution_and_examples() {
        let mut form = selection_form(&Options::default()).unwrap();
        form.fields[0].value = ENVIRONMENTS[1].into();
        constrain(&mut form);
        assert_eq!(form.fields[1].choices, ["manual"]);
        assert_eq!(form.fields[2].choices.len(), 3);
        form.fields[0].value = ENVIRONMENTS[2].into();
        constrain(&mut form);
        assert_eq!(form.fields[1].choices, ["runtime", "manual"]);
        assert_eq!(selected(&form).unwrap().environment, Environment::WasmThreads);
    }
    #[test]
    fn automatic_execution_follows_each_environment_default() {
        let mut form = selection_form(&Options::default()).unwrap();
        for (environment, expected) in [(1, "manual"), (2, "runtime"), (3, "manual"), (0, "runtime")] {
            form.fields[0].value = ENVIRONMENTS[environment].into();
            constrain(&mut form);
            assert_eq!(form.fields[1].value, expected);
            assert_eq!(form.fields[1].choices[0], expected);
        }
    }
    #[test]
    fn explicit_execution_survives_environment_changes_and_unavailable_choices() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut form = selection_form(&Options::default()).unwrap();
        form.selected = 1;
        form.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        constrain(&mut form);
        assert_eq!(form.fields[1].value, "manual");
        for environment in [1, 2, 3, 0] {
            form.fields[0].value = ENVIRONMENTS[environment].into();
            constrain(&mut form);
            assert_eq!(form.fields[1].value, "manual");
        }
        form.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        constrain(&mut form);
        assert_eq!(form.fields[1].value, "runtime");
        form.fields[0].value = ENVIRONMENTS[3].into();
        constrain(&mut form);
        assert_eq!(form.fields[1].value, "manual");
        form.fields[0].value = ENVIRONMENTS[0].into();
        constrain(&mut form);
        assert_eq!(form.fields[1].value, "runtime");
    }
    #[test]
    fn explicit_cli_selection_is_kept_and_case_preference_is_restored() {
        let mut form =
            selection_form(&Options { execution: Some(Execution::Manual), domain: Some(Domain::Movement), ..Default::default() }).unwrap();
        form.fields[0].value = ENVIRONMENTS[3].into();
        constrain(&mut form);
        assert_eq!(form.fields[2].value, "Client movement");
        form.fields[0].value = ENVIRONMENTS[0].into();
        constrain(&mut form);
        assert_eq!(form.fields[1].value, "manual");
        assert_eq!(form.fields[1].choices[0], "runtime");
        assert_eq!(form.fields[2].value, "Client movement");
    }
}
