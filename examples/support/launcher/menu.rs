//! Choose the example and environment before configuring the workload.
use super::{
    form::{Field, Form},
    selection::{Domain, Environment, Execution, Options, Selection},
};

const ENVIRONMENTS: [&str; 4] = ["Native", "WebAssembly / Node (WASI)", "WebAssembly / Wasmtime threads", "WebAssembly / Browser"];
const EXAMPLES: [&str; 2] = ["Game", "Benchmark"];
fn description(example: &str) -> &'static str {
    let readme = match example {
        "Game" => include_str!("../../cases/client_movement/README.md"),
        _ => include_str!("../../cases/chain_hash/README.md"),
    };
    readme
        .split_once("<!-- launcher:overview:start -->")
        .and_then(|(_, text)| text.split_once("<!-- launcher:overview:end -->"))
        .map_or("", |(overview, _)| overview.trim())
}
fn environment_description(value: &str) -> &'static str {
    let (readme, start, end) = match value {
        "Native" => (include_str!("../../environments/native/README.md"), "<!-- launcher:native:start -->", "<!-- launcher:native:end -->"),
        "WebAssembly / Node (WASI)" => (include_str!("wasi-environment.md"), "<!-- launcher:node:start -->", "<!-- launcher:node:end -->"),
        "WebAssembly / Wasmtime threads" => {
            (include_str!("wasi-environment.md"), "<!-- launcher:threads:start -->", "<!-- launcher:threads:end -->")
        }
        _ => (include_str!("browser-environment.md"), "<!-- launcher:browser:start -->", "<!-- launcher:browser:end -->"),
    };
    readme.split_once(start).and_then(|(_, text)| text.split_once(end)).map_or("", |(description, _)| description.trim())
}

fn update_description(form: &mut Form) {
    form.fields[0].help = description(&form.fields[0].value);
    form.fields[1].help = environment_description(&form.fields[1].value);
}
fn selection_form(options: &Options) -> Result<Form, String> {
    let selected = options.defaults()?;
    let mut example = Field::new(
        "Example",
        description(selected.domain.label()),
        selected.domain.label().into(),
        EXAMPLES.iter().map(|s| s.to_string()).collect(),
    );
    if options.domain.is_some() {
        example.remember_choice();
    }
    let environment = Field::new(
        "Environment",
        environment_description(selected.environment.label()),
        selected.environment.label().into(),
        ENVIRONMENTS.iter().map(|s| s.to_string()).collect(),
    );
    Ok(Form::new(vec![example, environment], false))
}
fn selected(form: &Form, options: &Options) -> Result<Selection, String> {
    let domain = match form.fields[0].value.as_str() {
        "Game" => Domain::Movement,
        _ => Domain::Hash,
    };
    let environment = match form.fields[1].value.as_str() {
        value if value == ENVIRONMENTS[0] => Environment::Terminal,
        value if value == ENVIRONMENTS[1] => Environment::Wasm,
        value if value == ENVIRONMENTS[2] => Environment::WasmThreads,
        _ => Environment::Browser,
    };
    let automatic =
        if matches!(environment, Environment::Terminal | Environment::WasmThreads) { Execution::Runtime } else { Execution::Manual };
    Selection::new(environment, if domain == Domain::Movement { automatic } else { options.execution.unwrap_or(automatic) }, domain)
}
pub fn selection(options: &Options, interactive: bool) -> Result<Option<Selection>, Box<dyn std::error::Error>> {
    if !interactive {
        return options.defaults().map(Some).map_err(Into::into);
    }
    let mut form = selection_form(options)?;
    if !form.run("Example launcher", "continue", update_description, |form| options.validate(selected(form, options)?))? {
        return Ok(None);
    }
    Ok(Some(selected(&form, options)?))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_descriptions_and_labels_fit_the_minimum_terminal() {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let mut form = selection_form(&Options::default()).unwrap();
        for (index, choices) in [(0, EXAMPLES.as_slice()), (1, ENVIRONMENTS.as_slice())] {
            for choice in choices {
                form.selected = index;
                form.fields[index].value = (*choice).into();
                update_description(&mut form);
                assert!(form.fields[index].help.contains("\n\n"), "{choice} needs two paragraphs");
                terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
                let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains("PgUp / PgDn"));
                assert!(text.contains(choice), "label clipped for {choice}");
            }
        }
        assert!(ENVIRONMENTS[1..].iter().all(|name| name.starts_with("WebAssembly / ")));
    }
    #[test]
    fn examples_display_relevant_readme_overviews() {
        for (choice, readme) in [
            ("Game", include_str!("../../cases/client_movement/README.md")),
            ("Benchmark", include_str!("../../cases/chain_hash/README.md")),
        ] {
            let help = description(choice);
            assert!(!help.is_empty(), "{choice} needs a README overview");
            assert!(readme.contains(help), "{choice} help must come from its README");
            assert!(!help.contains("cargo run"), "{choice} help should not include command examples");
            assert!(!help.contains("## "), "{choice} help should not include the full README");
            assert!(!help.contains("<!--"), "{choice} help should not expose README markers");
        }
    }
    #[test]
    fn long_example_description_can_be_scrolled_and_resets_on_selection_change() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut form = selection_form(&Options::default()).unwrap();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
        let initial: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        form.handle(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        update_description(&mut form);
        terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
        let scrolled: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        assert_ne!(initial, scrolled);
        for _ in 0..100 {
            form.handle(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
            terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
        }
        let end: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        let compact = |value: &str| value.chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        assert!(compact(&end).contains(&compact("There is no network server or client-side prediction.")));
        form.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        update_description(&mut form);
        terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
        let benchmark: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
        assert!(benchmark.contains("Benchmark measures an end-to-end event workload"));
    }
    #[test]
    fn environment_help_comes_from_the_relevant_guide() {
        for (name, readme) in [
            (ENVIRONMENTS[0], include_str!("../../environments/native/README.md")),
            (ENVIRONMENTS[1], include_str!("wasi-environment.md")),
            (ENVIRONMENTS[2], include_str!("wasi-environment.md")),
            (ENVIRONMENTS[3], include_str!("browser-environment.md")),
        ] {
            let description = environment_description(name);
            assert!(!description.is_empty());
            assert!(readme.contains(description), "{name} description must come from its guide");
            assert!(!description.contains("<!--"));
            assert!(description.contains("\n\n"));
        }
    }
    #[test]
    fn each_environment_description_can_be_read_to_the_end() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for width in [80, 120] {
            for choice in ENVIRONMENTS {
                let mut form = selection_form(&Options::default()).unwrap();
                form.selected = 1;
                form.fields[1].value = choice.into();
                update_description(&mut form);
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
                for _ in 0..50 {
                    terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
                    form.handle(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
                }
                terminal.draw(|frame| form.render(frame, "Example launcher", "continue")).unwrap();
                let text: String = terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect();
                let compact = |value: &str| value.chars().filter(|c| c.is_alphanumeric()).collect::<String>();
                let end = compact(environment_description(choice));
                assert!(compact(&text).contains(&end[end.len().saturating_sub(30)..]), "{choice} at {width} columns");
            }
        }
    }
    #[test]
    fn environment_help_describes_the_selected_host() {
        let mut form = selection_form(&Options::default()).unwrap();
        form.fields[1].value = "Native".into();
        update_description(&mut form);
        let native = form.fields[1].help;
        form.fields[1].value = "WebAssembly / Browser".into();
        update_description(&mut form);
        assert_ne!(native, form.fields[1].help);
        assert!(form.fields[1].help.contains("browser"));
    }
    #[test]
    fn each_example_starts_with_its_own_description() {
        let game = selection_form(&Options { domain: Some(Domain::Movement), ..Default::default() }).unwrap();
        let hash = selection_form(&Options { domain: Some(Domain::Hash), ..Default::default() }).unwrap();
        assert_ne!(game.fields[0].help, hash.fields[0].help);
    }
    #[test]
    fn changing_the_example_updates_its_description() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut form = selection_form(&Options::default()).unwrap();
        for expected in ["Benchmark", "Game"] {
            form.handle(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
            update_description(&mut form);
            assert_eq!(form.fields[0].value, expected);
            assert_eq!(form.fields[0].help, description(expected));
        }
    }
    #[test]
    fn example_is_first_and_environment_is_second() {
        let form = selection_form(&Options::default()).unwrap();
        assert_eq!(form.fields.len(), 2);
        assert_eq!((form.fields[0].label, form.fields[0].value.as_str()), ("Example", "Game"));
        assert_eq!((form.fields[1].label, form.fields[1].value.as_str()), ("Environment", "Native"));
    }
    #[test]
    fn default_execution_follows_the_environment() {
        let mut form = selection_form(&Options::default()).unwrap();
        for (index, expected) in [(0, Execution::Runtime), (1, Execution::Manual), (2, Execution::Runtime), (3, Execution::Manual)] {
            form.fields[1].value = ENVIRONMENTS[index].into();
            assert_eq!(selected(&form, &Options::default()).unwrap().execution, expected);
        }
    }
    #[test]
    fn explicit_hash_execution_is_checked_against_the_environment() {
        let options = Options { domain: Some(Domain::Hash), execution: Some(Execution::Runtime), ..Default::default() };
        let mut form = selection_form(&options).unwrap();
        form.fields[1].value = ENVIRONMENTS[3].into();
        assert!(selected(&form, &options).is_err());
        form.fields[1].value = ENVIRONMENTS[0].into();
        assert_eq!(selected(&form, &options).unwrap().execution, Execution::Runtime);
    }
}
