#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{BUFFER_PATH, LatencyRoutingSettings, PERCENTILE_PATH, Raw};
use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load_with(
    source: &str,
    variables: &[(&str, &str)],
) -> Result<LatencyRoutingSettings, ConfigError> {
    ConfigLoader {
        lookup: environment(variables),
    }
    .load(source)
    .map(|config| *config.sections.latency_routing)
}

// latency_observations converts first and refuses the same argument, so these cases exercise this section alone
fn convert_with(
    source: &str,
    variables: &[(&str, &str)],
) -> Result<LatencyRoutingSettings, ConfigError> {
    let (raw, _ignored) = parse_view::<Raw>(source)?;
    raw.convert(&environment(variables))
        .map(|settings| *settings)
}

fn load(source: &str) -> LatencyRoutingSettings {
    load_with(source, &[]).expect("the fixture config loads")
}

#[rstest]
#[case::absent("model_list: []\n", LatencyRoutingSettings::default())]
#[case::empty_args(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args: {}\n",
    LatencyRoutingSettings::default()
)]
#[case::both_arguments(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: 0.2\n    ttft_percentile: 0.9\n",
    LatencyRoutingSettings {
        lowest_latency_buffer: Some(0.2_f64),
        ttft_percentile: Some(0.9_f64),
    }
)]
#[case::percentile_of_one_is_allowed(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 1\n",
    LatencyRoutingSettings {
        lowest_latency_buffer: None,
        ttft_percentile: Some(1.0_f64),
    }
)]
// RoutingArgs declares no bound on the buffer, so a negative one is kept
#[case::a_negative_buffer_is_kept(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: -0.5\n",
    LatencyRoutingSettings {
        lowest_latency_buffer: Some(-0.5_f64),
        ttft_percentile: None,
    }
)]
#[case::null_keeps_the_default(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: null\n    ttft_percentile: null\n",
    LatencyRoutingSettings::default()
)]
#[case::another_strategy_keeps_the_defaults(
    "router_settings:\n  routing_strategy: simple-shuffle\n  routing_strategy_args:\n    lowest_latency_buffer: 0.2\n    ttft_percentile: 0.9\n",
    LatencyRoutingSettings::default()
)]
#[case::no_strategy_keeps_the_defaults(
    "router_settings:\n  routing_strategy_args:\n    lowest_latency_buffer: 0.2\n",
    LatencyRoutingSettings::default()
)]
fn converts(#[case] source: &str, #[case] expected: LatencyRoutingSettings) {
    assert_eq!(load(source), expected);
}

#[rstest]
#[case::percentile_zero(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 0\n",
    PERCENTILE_PATH
)]
#[case::percentile_above_one(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 1.5\n",
    PERCENTILE_PATH
)]
#[case::percentile_is_not_a_number(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: [0.5]\n",
    PERCENTILE_PATH
)]
#[case::buffer_is_no_number(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: [0.2]\n",
    BUFFER_PATH
)]
fn rejects(#[case] source: &str, #[case] path: &str) {
    let error = convert_with(source, &[]).expect_err("the fixture config is invalid");
    assert!(
        matches!(&error, ConfigError::InvalidScalar { path: named, .. } if named == path),
        "the conversion answered {error:?}"
    );
}

// the YAML layer refuses a non-finite literal before any section reads the value
#[test]
fn rejects_a_non_finite_percentile_literal() {
    let source = "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: .nan\n";
    let error = convert_with(source, &[]).expect_err("the fixture percentile is not finite");
    assert_eq!(error.category(), "yaml");
}

#[rstest]
#[case::environment_buffer(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: os.environ/ROUTER_BUFFER\n",
    &[("ROUTER_BUFFER", "0.3")],
    LatencyRoutingSettings {
        lowest_latency_buffer: Some(0.3_f64),
        ttft_percentile: None,
    }
)]
#[case::environment_percentile(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: os.environ/ROUTER_PERCENTILE\n",
    &[("ROUTER_PERCENTILE", "0.25")],
    LatencyRoutingSettings {
        lowest_latency_buffer: None,
        ttft_percentile: Some(0.25_f64),
    }
)]
fn resolves_environment(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: LatencyRoutingSettings,
) {
    assert_eq!(
        load_with(source, variables).expect("the fixture config loads"),
        expected
    );
}
