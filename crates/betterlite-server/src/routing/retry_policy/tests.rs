#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_router::cooldown_decisions::failure::FailureClass;
use betterlite_router::retry_policy::policy::RetryPolicy;

use super::{RetrySettings, resolve};

const EVERY_FIELD: &str = "\
router_settings:
  retry_after: 2
  num_retries: 3
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
  model_group_retry_policy:
    gpt-4:
      BadRequestErrorRetries: 11
      AuthenticationErrorRetries: 12
      TimeoutErrorRetries: 13
      RateLimitErrorRetries: 14
      ContentPolicyViolationErrorRetries: 15
      InternalServerErrorRetries: 16
      ServiceUnavailableErrorRetries: 17
      NotFoundErrorRetries: 18
      DefaultRetries: 19
";

fn settings(source: &str, variables: &[(&str, &str)]) -> RetrySettings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    resolve(&config.sections.retry_policy)
}

fn fixed(value: f64) -> String {
    format!("{value:.6}")
}

#[test]
fn an_empty_config_resolves_to_the_litellm_defaults() {
    let resolved = settings("router_settings: {}", &[]);
    assert_eq!(resolved.policies.global, None);
    assert!(resolved.policies.model_group.is_empty());
    assert_eq!(resolved.num_retries, 0);
    assert_eq!(fixed(resolved.backoff.initial_retry_delay), "0.500000");
    assert_eq!(fixed(resolved.backoff.max_retry_delay), "8.000000");
    assert_eq!(fixed(resolved.backoff.jitter), "0.750000");
    assert_eq!(fixed(resolved.backoff.min_timeout), "0.000000");
}

#[test]
fn the_configured_policies_and_backoff_reach_the_router_service() {
    let resolved = settings(EVERY_FIELD, &[("JITTER", "0.5")]);
    assert_eq!(
        resolved.policies.global,
        Some(RetryPolicy {
            bad_request: Some(1),
            authentication: Some(2),
            timeout: Some(3),
            rate_limit: Some(4),
            content_policy: Some(5),
            internal_server: Some(6),
            service_unavailable: Some(7),
            not_found: Some(8),
            fallback: Some(9),
        })
    );
    let group = RetryPolicy {
        bad_request: Some(11),
        authentication: Some(12),
        timeout: Some(13),
        rate_limit: Some(14),
        content_policy: Some(15),
        internal_server: Some(16),
        service_unavailable: Some(17),
        not_found: Some(18),
        fallback: Some(19),
    };
    assert_eq!(
        resolved.policies.model_group,
        BTreeMap::from([("gpt-4".to_owned(), group)])
    );
    assert_eq!(resolved.num_retries, 3);
    assert_eq!(fixed(resolved.backoff.min_timeout), "2.000000");
    assert_eq!(fixed(resolved.backoff.jitter), "0.500000");
}

#[test]
fn the_converted_policy_selects_like_the_reference() {
    let source = "\
router_settings:
  retry_policy:
    AuthenticationErrorRetries: 2
  model_group_retry_policy:
    gpt-4:
      DefaultRetries: 7
";
    let resolved = settings(source, &[]);
    let class = FailureClass::Authentication;
    let group_retries = resolved
        .policies
        .selected(Some("gpt-4"))
        .and_then(|policy| policy.retries_for(class, Some(401)));
    assert_eq!(group_retries, Some(7));
    let global_retries = resolved
        .policies
        .selected(None)
        .and_then(|policy| policy.retries_for(class, Some(401)));
    assert_eq!(global_retries, Some(2));
}
