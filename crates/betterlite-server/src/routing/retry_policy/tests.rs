#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_router::cooldown_decisions::failure::FailureClass;
use betterlite_router::retry_policy::policy::RetryPolicy;

use super::{RetrySettings, resolve};

fn settings(source: &str, variables: &[(&str, &str)]) -> RetrySettings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source).unwrap();
    resolve(&config.sections.retry_policy)
}

fn policy(retries: i64) -> RetryPolicy {
    RetryPolicy {
        bad_request: Some(retries),
        authentication: Some(retries),
        timeout: Some(retries),
        rate_limit: Some(retries),
        content_policy: Some(retries),
        internal_server: Some(retries),
        service_unavailable: Some(retries),
        not_found: Some(retries),
        fallback: Some(retries),
    }
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
    let source = "\
router_settings:
  retry_after: 2
  num_retries: 3
  retry_policy:
    DefaultRetries: 4
  model_group_retry_policy:
    gpt-4:
      RateLimitErrorRetries: 5
";
    let resolved = settings(source, &[("JITTER", "0.5")]);
    assert_eq!(resolved.policies.global, Some(policy(4)));
    let group = resolved.policies.model_group.get("gpt-4");
    assert_eq!(group.map(|policy| policy.rate_limit), Some(Some(5)));
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
