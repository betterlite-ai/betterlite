#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{Args, Settings};
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::RoutingStrategy;

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.lar1_routing)
}

fn load(source: &str) -> Settings {
    load_with(source, &[]).unwrap()
}

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn args(low: Option<&str>, medium: Option<&str>, high: Option<&str>) -> Option<Args> {
    Some(Args {
        low: low.map(dec),
        medium: medium.map(dec),
        high: high.map(dec),
    })
}

#[rstest]
#[case::absent("model_list: []\n", None)]
#[case::empty_args(
    "router_settings:\n  routing_strategy_args: {}\n",
    args(None, None, None)
)]
#[case::float_args(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: 0.35\n    confidence_threshold_medium: 0.45\n    confidence_threshold_high: 0.85\n",
    args(Some("0.35"), Some("0.45"), Some("0.85"))
)]
#[case::one_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_high: 0.9\n",
    args(None, None, Some("0.9"))
)]
#[case::integer_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_high: 1\n",
    args(None, None, Some("1"))
)]
#[case::null_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: null\n",
    args(None, None, None)
)]
#[case::text_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: low\n",
    args(None, None, None)
)]
#[case::numeric_text_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: \"0.2\"\n",
    args(None, None, None)
)]
#[case::boolean_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_medium: true\n",
    args(None, None, None)
)]
#[case::list_arg(
    "router_settings:\n  routing_strategy_args:\n    confidence_threshold_high: [0.9]\n",
    args(None, None, None)
)]
fn lar1_routing_args_load(#[case] source: &str, #[case] expected: Option<Args>) {
    assert_eq!(load(source).args, expected);
}

#[test]
fn lar1_routing_reads_an_environment_reference() {
    let source = "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: os.environ/LAR1_LOW\n";
    let settings = load_with(source, &[("LAR1_LOW", "0.35")]).unwrap();
    assert_eq!(settings.args, args(Some("0.35"), None, None));
}

#[test]
fn lar1_routing_treats_an_empty_environment_value_as_unset() {
    let source = "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: os.environ/LAR1_LOW\n";
    let settings = load_with(source, &[("LAR1_LOW", "")]).unwrap();
    assert_eq!(settings.args, args(None, None, None));
}

#[test]
fn lar1_routing_reports_a_missing_environment_variable() {
    let source = "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: os.environ/LAR1_LOW\n";
    let error = load_with(source, &[]).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "LAR1_LOW"
                && path == "router_settings.routing_strategy_args.confidence_threshold_low"
    ));
}

#[test]
fn lar1_routing_reports_a_non_numeric_environment_value() {
    let source = "router_settings:\n  routing_strategy_args:\n    confidence_threshold_low: os.environ/LAR1_LOW\n";
    let error = load_with(source, &[("LAR1_LOW", "low")]).unwrap_err();
    assert!(matches!(error, ConfigError::Environment { .. }));
}

#[test]
fn lar1_routing_rejects_an_unknown_argument() {
    let source =
        "router_settings:\n  routing_strategy_args:\n    confidence_threshold_ultra: 0.9\n";
    let error = load_with(source, &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("an unknown argument names its path");
    };
    assert_eq!(
        paths,
        vec!["router_settings.routing_strategy_args.confidence_threshold_ultra".to_owned()]
    );
}

#[test]
fn lar1_routing_loads_beside_the_routing_strategy() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("router_settings:\n  routing_strategy: lar1\n  routing_strategy_args:\n    confidence_threshold_low: 0.35\n")
    .unwrap();
    assert_eq!(
        config.sections.router_core.routing_strategy,
        RoutingStrategy::Lar1
    );
    assert_eq!(
        config.sections.lar1_routing.args,
        args(Some("0.35"), None, None)
    );
}
