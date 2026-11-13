#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::weighted_failover::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.weighted_failover)
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::absent("router_settings: {}", false)]
#[case::null("router_settings:\n  enable_weighted_failover: null", false)]
#[case::enabled("router_settings:\n  enable_weighted_failover: true", true)]
#[case::disabled("router_settings:\n  enable_weighted_failover: false", false)]
fn converts_the_weighted_failover_flag(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(settings(source, &[]).unwrap().enabled, expected);
}

#[rstest]
#[case::enabled("true", true)]
#[case::disabled("false", false)]
#[case::empty("", false)]
fn resolves_the_weighted_failover_reference(#[case] value: &str, #[case] expected: bool) {
    let source = "router_settings:\n  enable_weighted_failover: os.environ/WEIGHTED_FAILOVER";
    let loaded = settings(source, &[("WEIGHTED_FAILOVER", value)]).unwrap();
    assert_eq!(loaded.enabled, expected);
}

#[rstest]
#[case::bad_reference(
    "router_settings:\n  enable_weighted_failover: os.environ/WEIGHTED_FAILOVER",
    &[("WEIGHTED_FAILOVER", "maybe")],
    "WEIGHTED_FAILOVER"
)]
#[case::missing_reference(
    "router_settings:\n  enable_weighted_failover: os.environ/MISSING_FAILOVER",
    &[],
    "MISSING_FAILOVER"
)]
#[case::plain_text(
    "router_settings:\n  enable_weighted_failover: \"true\"",
    &[],
    "router_settings.enable_weighted_failover"
)]
#[case::number(
    "router_settings:\n  enable_weighted_failover: 1",
    &[],
    "router_settings.enable_weighted_failover"
)]
#[case::list(
    "router_settings:\n  enable_weighted_failover: [true]",
    &[],
    "router_settings.enable_weighted_failover"
)]
fn rejects_invalid_weighted_failover_settings(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] named: &str,
) {
    let error = settings(source, variables).unwrap_err();
    assert!(error.contains(named), "{error}");
}
