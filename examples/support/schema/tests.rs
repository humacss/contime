use super::*;
#[test]
fn definitions_validate_custom_values_and_presets_using_the_same_rules() {
    let settings = crate::support::Settings::parse([], false).unwrap().unwrap().0;
    let movement = crate::support::movement_config::Config::parse([]).unwrap().unwrap();
    for definitions in [common(&settings, false), super::movement(&movement), browser(1.0, 1000)] {
        let mut ids = std::collections::HashSet::new();
        for d in definitions {
            assert!(ids.insert(d.id));
            assert!(d.normalize(&d.default).is_ok(), "{}", d.id);
            for choice in &d.choices {
                assert!(d.normalize(choice).is_ok(), "{} {choice}", d.id);
            }
            assert!(d.normalize("").is_err());
            assert!(d.normalize("NaN").is_err());
        }
    }
}
#[test]
fn definitions_allow_custom_values_without_treating_presets_as_an_enum() {
    let d = browser(1.0, 1000).remove(0);
    assert_eq!(d.normalize("0.37").unwrap(), "0.37");
    assert!(d.normalize("0.009").is_err());
    let settings = crate::support::Settings::parse([], false).unwrap().unwrap().0;
    let defs = common(&settings, false);
    let average = defs.iter().find(|d| d.id == "--average-snapshots-per-event").unwrap();
    assert!(average.normalize("1.234").is_ok());
    assert!(average.normalize("1.2345").is_err());
    let worker = defs.iter().find(|d| d.id == "--workers").unwrap();
    assert!(worker.normalize("3").is_ok());
    assert!(worker.normalize("3.0").is_err());
}
