#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Args;
use crate::loader::{ConfigError, ConfigLoader, parse_view};
use crate::sections::Section as _;

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
    InvalidFloat,
    InvalidScalar,
    MissingEnvironment,
}

fn rejection(error: &ConfigError) -> Rejection {
    match error {
        ConfigError::Unsupported { .. } => Rejection::Unsupported,
        ConfigError::InvalidInteger { .. } => Rejection::InvalidInteger,
        ConfigError::InvalidFloat { .. } => Rejection::InvalidFloat,
        ConfigError::InvalidScalar { .. } => Rejection::InvalidScalar,
        ConfigError::MissingEnvironment { .. } => Rejection::MissingEnvironment,
        ConfigError::Environment { .. }
        | ConfigError::Yaml { .. }
        | ConfigError::LoadThread { .. }
        | ConfigError::CustomDimensions(_) => panic!("unexpected rejection {error:?}"),
    }
}

#[rstest]
#[case::absent("model_list: []\n", Args::default())]
#[case::empty_args(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args: {}\n",
    Args::default()
)]
#[case::all_args(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 120\n    lowest_latency_buffer: 0.2\n    max_latency_list_size: 5\n    ttft_percentile: 0.9\n",
    Args {
        ttl_seconds: Some(120.0_f64),
        lowest_latency_buffer: Some(0.2_f64),
        max_latency_list_size: Some(5),
        ttft_percentile: Some(0.9_f64),
    }
)]
#[case::whole_float_size(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: 10.0\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: Some(10),
        ttft_percentile: None,
    }
)]
#[case::negative_size(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: -1\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: Some(-1),
        ttft_percentile: None,
    }
)]
#[case::null_ttl_keeps_the_default(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: null\n",
    Args::default()
)]
#[case::percentile_of_one_is_allowed(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 1\n",
    Args {
        ttl_seconds: None,
        lowest_latency_buffer: None,
        max_latency_list_size: None,
        ttft_percentile: Some(1.0_f64),
    }
)]
#[case::whole_float_ttl(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 120.0\n",
    Args {
        ttl_seconds: Some(120.0_f64),
        lowest_latency_buffer: None,
        max_latency_list_size: None,
        ttft_percentile: None,
    }
)]
#[case::another_strategy_keeps_the_defaults(
    "router_settings:\n  routing_strategy: usage-based-routing-v2\n  routing_strategy_args:\n    ttl: 120\n    lowest_latency_buffer: [0.2]\n    max_latency_list_size: [5]\n    ttft_percentile: [0.9]\n",
    Args::default()
)]
#[case::no_strategy_keeps_the_defaults(
    "router_settings:\n  routing_strategy_args:\n    ttl: 120\n    ttft_percentile: 0\n",
    Args::default()
)]
fn converts(#[case] source: &str, #[case] expected: Args) {
    assert_eq!(load(source), expected);
}

#[rstest]
#[case::environment_ttl(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: os.environ/ROUTER_TTL\n",
    &[("ROUTER_TTL", "90")],
    Args {
        ttl_seconds: Some(90.0_f64),
        lowest_latency_buffer: None,
        max_latency_list_size: None,
        ttft_percentile: None,
    }
)]
#[case::environment_size(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: os.environ/ROUTER_SIZE\n",
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
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: 2.5\n",
    Rejection::InvalidInteger
)]
#[case::text_size(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: five\n",
    Rejection::InvalidInteger
)]
#[case::zero_percentile(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 0\n",
    Rejection::InvalidScalar
)]
#[case::percentile_above_one(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: 1.5\n",
    Rejection::InvalidScalar
)]
#[case::nan_percentile(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: \"nan\"\n",
    Rejection::InvalidScalar
)]
#[case::missing_environment(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: os.environ/ROUTER_BUFFER\n",
    Rejection::MissingEnvironment
)]
#[case::list_buffer(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    lowest_latency_buffer: [0.2]\n",
    Rejection::InvalidScalar
)]
#[case::list_size(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    max_latency_list_size: [5]\n",
    Rejection::InvalidScalar
)]
#[case::list_percentile(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttft_percentile: [0.9]\n",
    Rejection::InvalidScalar
)]
fn rejects(#[case] source: &str, #[case] expected: Rejection) {
    let error = load_with(source, &[]).expect_err("the section rejects the source");
    assert_eq!(rejection(&error), expected);
}

#[rstest]
#[case::list_ttl(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: [60]\n",
    Rejection::InvalidScalar
)]
#[case::words_ttl(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: soon\n",
    Rejection::InvalidFloat
)]
#[case::boolean_ttl(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: true\n",
    Rejection::InvalidScalar
)]
fn rejects_a_bad_ttl(#[case] source: &str, #[case] expected: Rejection) {
    let error = load_with(source, &[]).expect_err("the section rejects the ttl");
    assert_eq!(rejection(&error), expected);
    assert!(
        matches!(
            &error,
            ConfigError::InvalidFloat { path, .. } | ConfigError::InvalidScalar { path, .. }
                if path == "router_settings.routing_strategy_args.ttl"
        ),
        "the rejection names the ttl argument, got {error:?}"
    );
}

// the loader's usage_v2_accounting section refuses the fraction until B9940 gates it, so the section alone proves the float
#[test]
fn a_fractional_ttl_is_read_as_float_seconds() {
    let source = "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 1.5\n";
    let (raw, _ignored) = parse_view::<super::Raw>(source).expect("the section parses the source");
    let settings = raw
        .convert(&BTreeMap::<String, String>::new())
        .expect("the section accepts a fractional ttl");
    assert_eq!(settings.ttl_seconds, Some(1.5_f64));
}

#[rstest]
#[case::an_unknown_argument(
    "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 60\n    unknown_argument: 1\n",
    Rejection::Unsupported
)]
fn rejects_unknown_arguments(#[case] source: &str, #[case] expected: Rejection) {
    let error = load_with(source, &[]).expect_err("the section rejects the source");
    assert_eq!(rejection(&error), expected);
}

#[test]
fn the_lar1_thresholds_share_the_same_arguments_object() {
    let source = "router_settings:\n  routing_strategy: latency-based-routing\n  routing_strategy_args:\n    ttl: 60\n    confidence_threshold_high: 0.9\n";
    assert_eq!(load(source).ttl_seconds, Some(60.0_f64));
}
