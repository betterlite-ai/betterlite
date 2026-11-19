#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::usage_v2_accounting::Settings;

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.usage_v2_accounting)
}

fn load(source: &str) -> Settings {
    load_with(source, &[]).unwrap()
}

const ARGS: &str = "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: ";

#[rstest]
#[case::absent("model_list: []\n", 60)]
#[case::empty_args(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args: {}\n",
    60
)]
#[case::configured_seconds(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 120\n",
    120
)]
#[case::zero_is_used_as_given(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 0\n",
    0
)]
#[case::null_takes_the_default(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: null\n",
    60
)]
#[case::numeric_text(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: \"90\"\n",
    90
)]
fn converts_the_window_ttl(#[case] source: &str, #[case] seconds: u64) {
    assert_eq!(load(source).ttl, Duration::from_secs(seconds));
}

// Router._build_strategy_selector passes routing_strategy_args only to the v2 handler
#[rstest]
#[case::no_strategy_fraction("router_settings:\n  routing_strategy_args:\n    ttl: 1.5\n")]
#[case::no_strategy_words("router_settings:\n  routing_strategy_args:\n    ttl: words\n")]
#[case::no_strategy_list("router_settings:\n  routing_strategy_args:\n    ttl: [60]\n")]
#[case::simple_shuffle_fraction(
    "router_settings:\n  routing_strategy: simple-shuffle\n  routing_strategy_args:\n    ttl: 1.5\n"
)]
#[case::simple_shuffle_words(
    "router_settings:\n  routing_strategy: simple-shuffle\n  routing_strategy_args:\n    ttl: words\n"
)]
#[case::simple_shuffle_list(
    "router_settings:\n  routing_strategy: simple-shuffle\n  routing_strategy_args:\n    ttl: [60]\n"
)]
#[case::latency_fraction(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 1.5\n"
)]
#[case::latency_whole_seconds(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 120\n"
)]
#[case::usage_whole_seconds(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: 120\n"
)]
fn ignores_the_args_of_another_strategy(#[case] source: &str) {
    assert_eq!(load(source).ttl, Duration::from_secs(60));
}

#[rstest]
#[case::text("-1")]
#[case::fraction("1.5")]
#[case::words("soon")]
fn rejects_a_ttl_that_is_not_whole_seconds(#[case] value: &str) {
    let error = load_with(&format!("{ARGS}{value}\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidInteger { path, .. }
            if path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[rstest]
#[case::boolean("true")]
#[case::list("[60]")]
fn rejects_a_ttl_of_another_type(#[case] value: &str) {
    let error = load_with(&format!("{ARGS}{value}\n"), &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, .. }
            if path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[test]
fn reads_an_environment_reference() {
    let source = format!("{ARGS}os.environ/USAGE_V2_TTL\n");
    let settings = load_with(&source, &[("USAGE_V2_TTL", "30")]).unwrap();
    assert_eq!(settings.ttl, Duration::from_secs(30));
}

#[test]
fn treats_an_empty_environment_value_as_unset() {
    let source = format!("{ARGS}os.environ/USAGE_V2_TTL\n");
    let settings = load_with(&source, &[("USAGE_V2_TTL", "")]).unwrap();
    assert_eq!(settings.ttl, Duration::from_secs(60));
}

#[test]
fn reports_a_missing_environment_variable() {
    let source = format!("{ARGS}os.environ/USAGE_V2_TTL\n");
    let error = load_with(&source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "USAGE_V2_TTL"
                && path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[test]
fn reports_a_non_numeric_environment_value() {
    let source = format!("{ARGS}os.environ/USAGE_V2_TTL\n");
    let error = load_with(&source, &[("USAGE_V2_TTL", "soon")]).unwrap_err();
    assert!(matches!(error, ConfigError::Environment { .. }));
}

#[test]
fn rejects_an_unknown_argument() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl_window: 60\n";
    let error = load_with(source, &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unknown argument names its path");
    };
    assert_eq!(
        paths,
        vec!["router_settings.routing_strategy_args.?.ttl_window".to_owned()]
    );
}

#[test]
fn leaves_the_lar1_arguments_to_their_strategy() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(
        "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 45\n    confidence_threshold_low: 0.35\n",
    )
    .unwrap();
    assert_eq!(
        config.sections.usage_v2_accounting.ttl,
        Duration::from_secs(45)
    );
    assert!(config.sections.lar1_routing.args.is_none());
}
