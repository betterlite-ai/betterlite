#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{DEFAULT_IN_MEMORY_TTL, DEFAULT_POLLING_INTERVAL, Raw, Settings};
use crate::loader::{ConfigError, ConfigLoader, parse_view};

fn load_with(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.request_scheduler)
}

fn load(source: &str) -> Settings {
    load_with(source, &[]).expect("the fixture config loads")
}

#[rstest]
#[case::absent("model_list: []\n", Settings::default())]
#[case::empty_router_settings("router_settings: {}\n", Settings::default())]
#[case::null_keeps_the_defaults(
    "router_settings:\n  polling_interval: null\n  default_priority: null\n",
    Settings::default()
)]
#[case::a_configured_interval(
    "router_settings:\n  polling_interval: 0.25\n",
    Settings {
        polling_interval: 0.25_f64,
        default_priority: None,
        local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
    }
)]
// Scheduler.__init__ reads `polling_interval or DEFAULT_POLLING_INTERVAL`, so a configured zero takes the default
#[case::a_zero_interval_takes_the_default(
    "router_settings:\n  polling_interval: 0\n",
    Settings::default()
)]
#[case::a_negative_interval_is_kept(
    "router_settings:\n  polling_interval: -1\n",
    Settings {
        polling_interval: -1.0_f64,
        default_priority: None,
        local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
    }
)]
#[case::the_high_priority(
    "router_settings:\n  default_priority: 0\n",
    Settings {
        polling_interval: DEFAULT_POLLING_INTERVAL,
        default_priority: Some(0),
        local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
    }
)]
#[case::the_medium_priority(
    "router_settings:\n  default_priority: 128\n",
    Settings {
        polling_interval: DEFAULT_POLLING_INTERVAL,
        default_priority: Some(128),
        local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
    }
)]
#[case::the_low_priority(
    "router_settings:\n  default_priority: 255\n",
    Settings {
        polling_interval: DEFAULT_POLLING_INTERVAL,
        default_priority: Some(255),
        local_ttl_seconds: DEFAULT_IN_MEMORY_TTL,
    }
)]
fn converts(#[case] source: &str, #[case] expected: Settings) {
    assert_eq!(load(source), expected);
}

#[rstest]
#[case::a_priority_above_the_byte_range("router_settings:\n  default_priority: 256\n")]
#[case::a_negative_priority("router_settings:\n  default_priority: -1\n")]
#[case::a_priority_that_is_a_fraction("router_settings:\n  default_priority: 1.5\n")]
#[case::a_priority_that_is_text("router_settings:\n  default_priority: high\n")]
#[case::an_interval_that_is_text("router_settings:\n  polling_interval: fast\n")]
fn rejects(#[case] source: &str) {
    let error = load_with(source, &[]).expect_err("the fixture config is invalid");
    assert!(
        matches!(
            error,
            ConfigError::InvalidInteger { .. }
                | ConfigError::InvalidFloat { .. }
                | ConfigError::InvalidScalar { .. }
        ),
        "the conversion answered {error:?}"
    );
}

// a priority that is a list is not a number, so the YAML layer refuses it before any section reads the value
#[test]
fn rejects_a_priority_that_is_a_list() {
    let error = parse_view::<Raw>("router_settings:\n  default_priority: [128]\n")
        .expect_err("the fixture priority is a list");
    assert_eq!(error.category(), "yaml");
}

#[test]
fn a_priority_from_the_environment_is_kept() {
    let settings = load_with(
        "router_settings:\n  default_priority: os.environ/SCHEDULER_PRIORITY\n",
        &[("SCHEDULER_PRIORITY", "7")],
    )
    .expect("the fixture config loads");
    assert_eq!(settings.default_priority, Some(7));
}

// DEFAULT_POLLING_INTERVAL is a constant read once, and the configured interval wins over it
#[rstest]
#[case::the_environment_default("model_list: []\n", &[("DEFAULT_POLLING_INTERVAL", "0.5")], 0.5_f64)]
#[case::a_configured_interval_wins(
    "router_settings:\n  polling_interval: 0.25\n",
    &[("DEFAULT_POLLING_INTERVAL", "0.5")],
    0.25_f64
)]
#[case::a_zero_interval_falls_back_to_the_environment(
    "router_settings:\n  polling_interval: 0\n",
    &[("DEFAULT_POLLING_INTERVAL", "0.5")],
    0.5_f64
)]
#[case::an_empty_environment_default(
    "model_list: []\n",
    &[("DEFAULT_POLLING_INTERVAL", "")],
    DEFAULT_POLLING_INTERVAL
)]
#[case::no_environment_default("model_list: []\n", &[], DEFAULT_POLLING_INTERVAL)]
#[case::a_configured_environment_reference(
    "router_settings:\n  polling_interval: os.environ/SCHEDULER_INTERVAL\n",
    &[("SCHEDULER_INTERVAL", "0.5")],
    0.5_f64
)]
fn polling_interval_reads_the_environment_default(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: f64,
) {
    let settings = load_with(source, variables).expect("the fixture config loads");
    assert_eq!(settings.polling_interval.to_bits(), expected.to_bits());
}

#[test]
fn local_ttl_reads_the_environment_default() {
    let configured = load_with("model_list: []\n", &[("DEFAULT_IN_MEMORY_TTL", "2")])
        .expect("the fixture config loads");
    assert_eq!(
        configured.local_ttl_seconds.to_bits(),
        2.0_f64.to_bits(),
        "a configured ttl"
    );
    let empty = load_with("model_list: []\n", &[("DEFAULT_IN_MEMORY_TTL", "")])
        .expect("the fixture config loads");
    assert_eq!(
        empty.local_ttl_seconds.to_bits(),
        DEFAULT_IN_MEMORY_TTL.to_bits(),
        "an empty ttl"
    );
    let absent = load_with("model_list: []\n", &[]).expect("the fixture config loads");
    assert_eq!(
        absent.local_ttl_seconds.to_bits(),
        DEFAULT_IN_MEMORY_TTL.to_bits(),
        "an absent ttl"
    );
}
