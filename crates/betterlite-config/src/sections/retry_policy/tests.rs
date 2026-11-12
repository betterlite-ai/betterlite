#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::retry_policy::{RetryPolicySettings, Settings};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.retry_policy)
        .map_err(|error| error.to_string())
}

fn empty_policy() -> RetryPolicySettings {
    RetryPolicySettings {
        bad_request: None,
        authentication: None,
        timeout: None,
        rate_limit: None,
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: None,
    }
}

fn fixed(value: f64) -> String {
    format!("{value:.6}")
}

#[test]
fn an_empty_config_takes_the_declaration_defaults() {
    let loaded = settings("router_settings: {}", &[]).unwrap();
    assert_eq!(loaded, Settings::default());
    assert_eq!(loaded.num_retries, 2);
    assert_eq!(fixed(loaded.retry_after), "0.000000");
    assert_eq!(fixed(loaded.initial_retry_delay), "0.500000");
    assert_eq!(fixed(loaded.max_retry_delay), "8.000000");
    assert_eq!(fixed(loaded.jitter), "0.750000");
    assert_eq!(loaded.retry_policy, None);
    assert!(loaded.model_group_retry_policy.is_empty());
}

#[rstest]
#[case::initial("INITIAL_RETRY_DELAY", "1.5", "1.500000")]
#[case::maximum("MAX_RETRY_DELAY", "9", "9.000000")]
#[case::jitter("JITTER", "0.25", "0.250000")]
fn the_environment_replaces_a_backoff_default(
    #[case] name: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let loaded = settings("router_settings: {}", &[(name, value)]).unwrap();
    let actual = match name {
        "INITIAL_RETRY_DELAY" => loaded.initial_retry_delay,
        "MAX_RETRY_DELAY" => loaded.max_retry_delay,
        _ => loaded.jitter,
    };
    assert_eq!(fixed(actual), expected);
}

#[rstest]
#[case::initial("INITIAL_RETRY_DELAY")]
#[case::maximum("MAX_RETRY_DELAY")]
#[case::jitter("JITTER")]
fn an_empty_environment_value_is_unset(#[case] name: &str) {
    let loaded = settings("router_settings: {}", &[(name, "")]).unwrap();
    assert_eq!(loaded, Settings::default());
}

#[rstest]
#[case::initial("INITIAL_RETRY_DELAY")]
#[case::maximum("MAX_RETRY_DELAY")]
#[case::jitter("JITTER")]
fn an_invalid_environment_value_names_the_variable(#[case] name: &str) {
    let error = settings("router_settings: {}", &[(name, "soon")]).unwrap_err();
    assert!(error.contains(name), "got {error}");
}

#[test]
fn every_policy_field_is_read_from_the_config_names() {
    let source = "\
router_settings:
  retry_policy:
    BadRequestErrorRetries: 1
    AuthenticationErrorRetries: 2
    TimeoutErrorRetries: 3
    RateLimitErrorRetries: 4
    ContentPolicyViolationErrorRetries: 5
    InternalServerErrorRetries: 6
    ServiceUnavailableErrorRetries: 7
    NotFoundErrorRetries: 8
    DefaultRetries: 9
";
    let loaded = settings(source, &[]).unwrap();
    let expected = RetryPolicySettings {
        bad_request: Some(1),
        authentication: Some(2),
        timeout: Some(3),
        rate_limit: Some(4),
        content_policy: Some(5),
        internal_server: Some(6),
        service_unavailable: Some(7),
        not_found: Some(8),
        fallback: Some(9),
    };
    assert_eq!(loaded.retry_policy, Some(expected));
}

#[test]
fn a_partial_policy_keeps_unset_fields_absent() {
    let source = "\
router_settings:
  retry_policy:
    RateLimitErrorRetries: 3
";
    let loaded = settings(source, &[]).unwrap();
    let expected = RetryPolicySettings {
        bad_request: None,
        authentication: None,
        timeout: None,
        rate_limit: Some(3),
        content_policy: None,
        internal_server: None,
        service_unavailable: None,
        not_found: None,
        fallback: None,
    };
    assert_eq!(loaded.retry_policy, Some(expected));
}

#[test]
fn a_null_policy_and_null_fields_take_their_defaults() {
    let source = "\
router_settings:
  retry_policy:
    RateLimitErrorRetries: null
    DefaultRetries: 2
";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(
        loaded.retry_policy,
        Some(RetryPolicySettings {
            bad_request: None,
            authentication: None,
            timeout: None,
            rate_limit: None,
            content_policy: None,
            internal_server: None,
            service_unavailable: None,
            not_found: None,
            fallback: Some(2),
        })
    );
    let null_policy = settings("router_settings:\n  retry_policy: null\n", &[]).unwrap();
    assert_eq!(null_policy.retry_policy, None);
}

#[test]
fn a_model_group_keeps_its_own_policy() {
    let source = "\
router_settings:
  retry_policy:
    DefaultRetries: 1
  model_group_retry_policy:
    gpt-4:
      RateLimitErrorRetries: 4
    claude:
      DefaultRetries: 2
";
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(
        loaded.retry_policy,
        Some(RetryPolicySettings {
            fallback: Some(1),
            ..empty_policy()
        })
    );
    let mut expected = BTreeMap::new();
    expected.insert(
        "gpt-4".to_owned(),
        RetryPolicySettings {
            rate_limit: Some(4),
            ..empty_policy()
        },
    );
    expected.insert(
        "claude".to_owned(),
        RetryPolicySettings {
            fallback: Some(2),
            ..empty_policy()
        },
    );
    assert_eq!(loaded.model_group_retry_policy, expected);
}

#[test]
fn a_policy_field_can_come_from_the_environment() {
    let source = "\
router_settings:
  retry_policy:
    RateLimitErrorRetries: os.environ/RATE_LIMIT_RETRIES
";
    let loaded = settings(source, &[("RATE_LIMIT_RETRIES", "6")]).unwrap();
    let retries = loaded.retry_policy.map(|policy| policy.rate_limit);
    assert_eq!(retries, Some(Some(6)));
    let missing = settings(source, &[]).unwrap_err();
    assert!(missing.contains("RATE_LIMIT_RETRIES"), "got {missing}");
}

#[rstest]
#[case::zero("0", 0)]
#[case::positive("3", 3)]
fn num_retries_is_read_as_an_integer(#[case] value: &str, #[case] expected: i64) {
    let source = format!("router_settings:\n  num_retries: {value}\n");
    let loaded = settings(&source, &[]).unwrap();
    assert_eq!(loaded.num_retries, expected);
}

// the shared router and litellm settings hold the count as u32, so the load fails before this section reads it
#[rstest]
#[case::router("router_settings:\n  num_retries: -1\n", "router_settings.num_retries")]
#[case::litellm(
    "litellm_settings:\n  num_retries: -1\n",
    "litellm_settings.num_retries"
)]
fn a_negative_num_retries_fails_the_whole_load_naming_the_field(
    #[case] source: &str,
    #[case] path: &str,
) {
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains(path), "got {error}");
}

#[rstest]
#[case::absent("router_settings: {}\n", 2)]
#[case::litellm_only("router_settings: {}\nlitellm_settings:\n  num_retries: 3\n", 3)]
#[case::router_only("router_settings:\n  num_retries: 3\n", 3)]
#[case::router_beats_litellm(
    "router_settings:\n  num_retries: 4\nlitellm_settings:\n  num_retries: 3\n",
    4
)]
fn the_retry_count_reads_router_then_litellm_then_the_default(
    #[case] source: &str,
    #[case] expected: i64,
) {
    let loaded = settings(source, &[]).unwrap();
    assert_eq!(loaded.num_retries, expected);
}

#[test]
fn an_invalid_num_retries_names_the_field() {
    let error = settings("router_settings:\n  num_retries: three\n", &[]).unwrap_err();
    assert!(error.contains("router_settings.num_retries"), "got {error}");
    let fraction = settings("router_settings:\n  num_retries: 1.5\n", &[]).unwrap_err();
    assert!(
        fraction.contains("router_settings.num_retries"),
        "got {fraction}"
    );
}

#[test]
fn a_configured_retry_after_is_kept() {
    let loaded = settings("router_settings:\n  retry_after: 5\n", &[]).unwrap();
    assert_eq!(fixed(loaded.retry_after), "5.000000");
    let zero = settings("router_settings:\n  retry_after: 0\n", &[]).unwrap();
    assert_eq!(fixed(zero.retry_after), "0.000000");
}

#[test]
fn an_invalid_policy_value_names_its_field_and_group() {
    let error = settings(
        "router_settings:\n  model_group_retry_policy:\n    gpt-4:\n      RateLimitErrorRetries: many\n",
        &[],
    )
    .unwrap_err();
    assert!(
        error.contains("router_settings.model_group_retry_policy.gpt-4.RateLimitErrorRetries"),
        "got {error}"
    );
}

#[rstest]
#[case::global("router_settings:\n  retry_policy:\n    RateLimitErrorRetry: 3\n")]
#[case::group(
    "router_settings:\n  model_group_retry_policy:\n    gpt-4:\n      RateLimitErrorRetry: 3\n"
)]
fn an_unknown_policy_key_fails_with_its_name(#[case] source: &str) {
    let error = settings(source, &[]).unwrap_err();
    assert!(error.contains("RateLimitErrorRetry"), "got {error}");
}

#[test]
fn a_null_group_map_keeps_the_global_policy() {
    let source = "\
router_settings:
  retry_policy:
    DefaultRetries: 2
  model_group_retry_policy: null
";
    let loaded = settings(source, &[]).unwrap();
    assert!(loaded.model_group_retry_policy.is_empty());
    assert_eq!(
        loaded.retry_policy,
        Some(RetryPolicySettings {
            fallback: Some(2),
            ..empty_policy()
        })
    );
}
