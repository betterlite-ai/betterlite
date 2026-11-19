#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{Settings, UsageRoutingSettings};
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::RoutingStrategy;

fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: lookup(variables),
    }
    .load(source)
    .map(|config| config.sections.usage_routing)
}

fn load(source: &str) -> UsageRoutingSettings {
    *load_with(source, &[]).unwrap()
}

fn ttl(source: &str) -> Option<i64> {
    load(source).ttl_seconds
}

#[rstest]
#[case::absent("model_list: []\n", None)]
#[case::empty_args(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args: {}\n",
    None
)]
#[case::integer(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: 45\n",
    Some(45)
)]
#[case::zero(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: 0\n",
    Some(0)
)]
#[case::whole_float(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: 45.0\n",
    Some(45)
)]
#[case::numeric_text(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: \"45\"\n",
    Some(45)
)]
#[case::null(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: null\n",
    None
)]
#[case::null_args(
    "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args: null\n",
    None
)]
fn usage_routing_reads_the_declared_ttl(#[case] source: &str, #[case] expected: Option<i64>) {
    assert_eq!(ttl(source), expected);
}

// Router._build_strategy_selector passes routing_strategy_args only to the legacy usage handler
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
#[case::usage_v2_whole_seconds(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 120\n"
)]
#[case::usage_v2_whole_float(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 120.0\n"
)]
fn ignores_the_args_of_another_strategy(#[case] source: &str) {
    assert_eq!(ttl(source), None);
}

// the shared key holds whole non-negative seconds, so text, a fraction and a negative name the key's path
#[rstest]
#[case::text("text")]
#[case::fractional("1.5")]
#[case::empty("\"\"")]
#[case::negative("-1")]
fn usage_routing_rejects_a_ttl_it_cannot_read(#[case] value: &str) {
    let source = format!(
        "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: {value}\n"
    );
    let error = load_with(&source, &[]).unwrap_err();
    assert!(
        matches!(
            error,
            ConfigError::InvalidInteger { path, .. }
                if path == "router_settings.routing_strategy_args.ttl"
        ),
        "a non-integer ttl names its path"
    );
}

// a list is not the int RoutingArgs declares, so the value must reach convert and be named there
#[test]
fn usage_routing_rejects_a_ttl_of_another_type() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: [60]\n";
    let error = load_with(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, value: false }
            if path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[test]
fn usage_routing_rejects_a_boolean_ttl() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: true\n";
    let error = load_with(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::InvalidScalar { path, value: true }
            if path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[test]
fn usage_routing_reads_an_environment_reference() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: os.environ/USAGE_TTL\n";
    let settings = load_with(source, &[("USAGE_TTL", "90")]).unwrap();
    assert_eq!(settings.ttl_seconds, Some(90));
}

#[test]
fn usage_routing_treats_an_empty_environment_value_as_unset() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: os.environ/USAGE_TTL\n";
    let settings = load_with(source, &[("USAGE_TTL", "")]).unwrap();
    assert_eq!(settings.ttl_seconds, None);
}

#[test]
fn usage_routing_reports_a_missing_environment_variable() {
    let source = "router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: os.environ/USAGE_TTL\n";
    let error = load_with(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "USAGE_TTL"
                && path == "router_settings.routing_strategy_args.ttl"
    ));
}

#[test]
fn usage_routing_loads_beside_the_routing_strategy() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("router_settings:\n  routing_strategy: usage-based-routing\n  routing_strategy_args:\n    ttl: 45\n")
    .unwrap();
    assert_eq!(
        config.sections.router_core.routing_strategy,
        RoutingStrategy::UsageBasedRouting
    );
    assert_eq!(config.sections.usage_routing.ttl_seconds, Some(45));
}
