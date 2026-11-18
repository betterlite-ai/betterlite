#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Args;
use crate::loader::{ConfigError, ConfigLoader};

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Args, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| *config.sections.latency_observations)
}

fn load(source: &str) -> Args {
    load_with(source, &[]).unwrap()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rejection {
    Unsupported,
    InvalidInteger,
    InvalidScalar,
    MissingEnvironment,
}

fn rejection(error: &ConfigError) -> Rejection {
    match error {
        ConfigError::Unsupported { .. } => Rejection::Unsupported,
        ConfigError::InvalidInteger { .. } => Rejection::InvalidInteger,
        ConfigError::InvalidScalar { .. } => Rejection::InvalidScalar,
        ConfigError::MissingEnvironment { .. } => Rejection::MissingEnvironment,
        ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::InvalidFloat { .. }
        | ConfigError::LoadThread { .. } => panic!("unexpected rejection {error:?}"),
    }
}

#[rstest]
#[case::absent("model_list: []\n", Args::default())]
#[case::empty_args("router_settings:\n  routing_strategy_args: {}\n", Args::default())]
#[case::all_args(
    "router_settings:\n  routing_strategy_args:\n    ttl: 120\n    lowest_latency_buffer: 0.2\n    max_latency_list_size: 5\n    ttft_percentile: 0.9\n",
    Args {
        ttl_seconds: Some(120.0_f64),
        lowest_latency_buffer: Some(0.2_f64),
        max_latency_list_size: Some(5),
        ttft_percentile: Some(0.9_f64),
    }
)]
#[case::whole_float_size(
    "router_settings:\n  routing_strategy_args:\n    max_latency_list_size: 10.0\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: Some(10),
        ttft_percentile: None,
    }
)]
#[case::negative_size(
    "router_settings:\n  routing_strategy_args:\n    max_latency_list_size: -1\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: Some(-1),
        ttft_percentile: None,
    }
)]
#[case::null_ttl_keeps_the_default(
    "router_settings:\n  routing_strategy_args:\n    ttl: null\n",
    Args::default()
)]
#[case::percentile_of_one_is_allowed(
    "router_settings:\n  routing_strategy_args:\n    ttft_percentile: 1\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: None,
        ttft_percentile: Some(1.0_f64),
    }
)]
fn converts(#[case] source: &str, #[case] expected: Args) {
    assert_eq!(load(source), expected);
}

#[rstest]
#[case::environment_ttl(
    "router_settings:\n  routing_strategy_args:\n    ttl: os.environ/ROUTER_TTL\n",
    &[("ROUTER_TTL", "90")],
    Args {
        ttl_seconds: Some(90.0_f64),
        lowest_latency_buffer: None,
        max_latency_list_size: None,
        ttft_percentile: None,
    }
)]
#[case::environment_size(
    "router_settings:\n  routing_strategy_args:\n    max_latency_list_size: os.environ/ROUTER_SIZE\n",
    &[("ROUTER_SIZE", "4")],
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: Some(4),
        ttft_percentile: None,
    }
)]
fn resolves_environment(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Args,
) {
    assert_eq!(load_with(source, variables).unwrap(), expected);
}

#[rstest]
#[case::fractional_size(
    "router_settings:\n  routing_strategy_args:\n    max_latency_list_size: 2.5\n",
    Rejection::InvalidInteger
)]
#[case::text_size(
    "router_settings:\n  routing_strategy_args:\n    max_latency_list_size: five\n",
    Rejection::InvalidInteger
)]
#[case::zero_percentile(
    "router_settings:\n  routing_strategy_args:\n    ttft_percentile: 0\n",
    Rejection::InvalidScalar
)]
#[case::percentile_above_one(
    "router_settings:\n  routing_strategy_args:\n    ttft_percentile: 1.5\n",
    Rejection::InvalidScalar
)]
#[case::nan_percentile(
    "router_settings:\n  routing_strategy_args:\n    ttft_percentile: \"nan\"\n",
    Rejection::InvalidScalar
)]
#[case::missing_environment(
    "router_settings:\n  routing_strategy_args:\n    ttl: os.environ/ROUTER_TTL\n",
    Rejection::MissingEnvironment
)]
fn rejects(#[case] source: &str, #[case] expected: Rejection) {
    let error = load_with(source, &[]).expect_err("the section rejects the source");
    assert_eq!(rejection(&error), expected);
}

#[rstest]
#[case::an_unknown_argument(
    "router_settings:\n  routing_strategy_args:\n    ttl: 60\n    unknown_argument: 1\n",
    Rejection::Unsupported
)]
fn rejects_unknown_arguments(#[case] source: &str, #[case] expected: Rejection) {
    let error = load_with(source, &[]).expect_err("the section rejects the source");
    assert_eq!(rejection(&error), expected);
}

#[test]
fn the_lar1_thresholds_share_the_same_arguments_object() {
    let source = "router_settings:\n  routing_strategy_args:\n    ttl: 60\n    confidence_threshold_high: 0.9\n";
    assert_eq!(load(source).ttl_seconds, Some(60.0_f64));
}
