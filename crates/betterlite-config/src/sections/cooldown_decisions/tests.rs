#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::cooldown_decisions::{AllowedFailsPolicy, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.cooldown_decisions)
        .map_err(|error| error.to_string())
}

#[rstest]
#[case::absent_defaults("router_settings: {}", &[], (3_u32, 3_u32, 5.0_f64, false))]
#[case::environment_defaults(
    "router_settings: {}",
    &[
        ("DEFAULT_ALLOWED_FAILS", "7"),
        ("DEFAULT_COOLDOWN_TIME_SECONDS", "9")
    ],
    (7_u32, 7_u32, 9.0_f64, false)
)]
#[case::empty_environment_is_unset(
    "router_settings: {}",
    &[
        ("DEFAULT_ALLOWED_FAILS", ""),
        ("DEFAULT_COOLDOWN_TIME_SECONDS", "")
    ],
    (3_u32, 3_u32, 5.0_f64, false)
)]
#[case::configured_allowed_fails(
    "router_settings:\n  allowed_fails: 2",
    &[],
    (2_u32, 3_u32, 5.0_f64, false)
)]
#[case::configured_cooldown_time(
    "router_settings:\n  cooldown_time: 7.5",
    &[],
    (3_u32, 3_u32, 7.5_f64, false)
)]
#[case::zero_cooldown_time_is_used_as_given(
    "router_settings:\n  cooldown_time: 0",
    &[],
    (3_u32, 3_u32, 0.0_f64, false)
)]
#[case::environment_reference(
    "router_settings:\n  allowed_fails: os.environ/FAILS",
    &[("FAILS", "4")],
    (4_u32, 3_u32, 5.0_f64, false)
)]
#[case::disable_cooldowns(
    "router_settings:\n  disable_cooldowns: true",
    &[],
    (3_u32, 3_u32, 5.0_f64, true)
)]
fn converts_router_cooldown_settings(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: (u32, u32, f64, bool),
) {
    let loaded = settings(source, variables).unwrap();
    let (allowed_fails, default_allowed_fails, cooldown_time, disable_cooldowns) = expected;
    assert_eq!(loaded.allowed_fails, allowed_fails);
    assert_eq!(loaded.default_allowed_fails, default_allowed_fails);
    assert_eq!(loaded.cooldown_time.to_bits(), cooldown_time.to_bits());
    assert_eq!(loaded.disable_cooldowns, disable_cooldowns);
    assert_eq!(loaded.allowed_fails_policy, None);
    assert_eq!(loaded.failure_ratio_threshold.to_bits(), 0.5_f64.to_bits());
    assert_eq!(loaded.minimum_requests, 5);
    assert_eq!(loaded.single_deployment_traffic, 1000);
}

#[test]
fn reads_every_allowed_fails_policy_field() {
    let source = "\
router_settings:
  allowed_fails_policy:
    ContentPolicyViolationErrorAllowedFails: 1
    BadRequestErrorAllowedFails: 2
    AuthenticationErrorAllowedFails: 3
    TimeoutErrorAllowedFails: 4
    RateLimitErrorAllowedFails: 5
    InternalServerErrorAllowedFails: 6
    ServiceUnavailableErrorAllowedFails: 7
    BadGatewayErrorAllowedFails: 8
    NotFoundErrorAllowedFails: 9
";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(
        loaded.allowed_fails_policy,
        Some(AllowedFailsPolicy {
            content_policy_violation: Some(1),
            bad_request: Some(2),
            authentication: Some(3),
            timeout: Some(4),
            rate_limit: Some(5),
            internal_server: Some(6),
            service_unavailable: Some(7),
            bad_gateway: Some(8),
            not_found: Some(9),
        })
    );
}

#[test]
fn reads_the_failure_threshold_environment() {
    let loaded = settings(
        "router_settings: {}",
        &[
            ("DEFAULT_FAILURE_THRESHOLD_PERCENT", "0.25"),
            ("DEFAULT_FAILURE_THRESHOLD_MINIMUM_REQUESTS", "2"),
            ("SINGLE_DEPLOYMENT_TRAFFIC_FAILURE_THRESHOLD", "10"),
        ],
    )
    .unwrap();
    assert_eq!(loaded.failure_ratio_threshold.to_bits(), 0.25_f64.to_bits());
    assert_eq!(loaded.minimum_requests, 2);
    assert_eq!(loaded.single_deployment_traffic, 10);
}

#[rstest]
#[case::bad_environment("router_settings: {}", &[("DEFAULT_ALLOWED_FAILS", "many")])]
#[case::bad_policy_text(
    "router_settings:\n  allowed_fails_policy:\n    BadRequestErrorAllowedFails: many",
    &[]
)]
#[case::bad_policy_float(
    "router_settings:\n  allowed_fails_policy:\n    BadRequestErrorAllowedFails: 1.5",
    &[]
)]
#[case::unknown_policy_key(
    "router_settings:\n  allowed_fails_policy:\n    RetryPolicyAllowedFails: 1",
    &[]
)]
fn rejects_invalid_cooldown_settings(#[case] source: &str, #[case] variables: &[(&str, &str)]) {
    settings(source, variables).unwrap_err();
}

#[rstest]
#[case::allowed_fails(
    "router_settings:\n  allowed_fails: true",
    "router_settings.allowed_fails"
)]
#[case::cooldown_time(
    "router_settings:\n  cooldown_time: true",
    "router_settings.cooldown_time"
)]
fn names_the_router_setting_path(#[case] source: &str, #[case] path: &str) {
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains(path), "{error}");
}
