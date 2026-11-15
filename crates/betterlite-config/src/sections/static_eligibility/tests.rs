#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn loaded(source: &str) -> bool {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
    .sections
    .static_eligibility
    .enable_pre_call_checks
}

fn loaded_from(source: &str, environment: &[(&str, &str)]) -> Result<bool, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .static_eligibility
        .enable_pre_call_checks)
}

#[rstest]
#[case::absent("router_settings: {}\n", false)]
#[case::off("router_settings:\n  enable_pre_call_checks: false\n", false)]
#[case::on("router_settings:\n  enable_pre_call_checks: true\n", true)]
fn static_eligibility_enable_pre_call_checks_loads(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(loaded(source), expected);
}

#[test]
fn static_eligibility_defaults_when_router_settings_are_absent() {
    assert!(!loaded("model_list: []\n"));
}

#[rstest]
#[case::on("on", true)]
#[case::one("1", true)]
#[case::off("off", false)]
#[case::zero("0", false)]
#[case::empty_counts_as_unset("", false)]
fn static_eligibility_enable_pre_call_checks_reads_the_environment(
    #[case] value: &str,
    #[case] expected: bool,
) {
    let source = "router_settings:\n  enable_pre_call_checks: os.environ/PRE_CALL_CHECKS\n";
    assert_eq!(
        loaded_from(source, &[("PRE_CALL_CHECKS", value)]).unwrap(),
        expected
    );
}

#[test]
fn static_eligibility_enable_pre_call_checks_reports_the_environment() {
    let source = "router_settings:\n  enable_pre_call_checks: os.environ/PRE_CALL_CHECKS\n";
    let missing = loaded_from(source, &[]).unwrap_err();
    assert!(matches!(
        &missing,
        ConfigError::MissingEnvironment { variable, .. } if variable == "PRE_CALL_CHECKS"
    ));
    let unreadable = loaded_from(source, &[("PRE_CALL_CHECKS", "sometimes")]).unwrap_err();
    assert!(matches!(unreadable, ConfigError::Environment { .. }));
    assert_eq!(
        unreadable.to_string(),
        "environment variable PRE_CALL_CHECKS must be a bool"
    );
}
