use contime_progress::CoverageConstraint;

#[test]
fn all_routes_must_be_covered_before_replacing_the_fallback() {
    let mut constraint = CoverageConstraint::new(20, [1, 2]);
    assert_eq!(constraint.boundary(), Some(20));
    constraint.cover(1, Some(90));
    assert_eq!(constraint.boundary(), Some(20));
    constraint.cover(2, Some(100));
    assert_eq!(constraint.boundary(), Some(90));
}

#[test]
fn covered_replay_before_the_fallback_is_preserved() {
    let mut constraint = CoverageConstraint::new(20, [1, 2]);
    constraint.cover(1, Some(10));
    assert_eq!(constraint.boundary(), Some(10));
    constraint.cover(2, None);
    assert_eq!(constraint.boundary(), Some(10));
}

#[test]
fn duplicate_and_unknown_routes_cannot_discharge_another_route() {
    let mut constraint = CoverageConstraint::new(20, [1, 2]);
    constraint.cover(1, Some(90));
    constraint.cover(1, None);
    constraint.cover(3, None);
    assert_eq!(constraint.boundary(), Some(20));
    constraint.cover(2, None);
    assert_eq!(constraint.boundary(), Some(90));
}

#[test]
fn no_work_and_no_routes_do_not_constrain_the_observation() {
    let mut constraint = CoverageConstraint::new(20, [1]);
    constraint.cover(1, None);
    assert_eq!(constraint.boundary(), None);
    assert_eq!(CoverageConstraint::new(20, [] as [u128; 0]).boundary(), None);
}
