use std::collections::BTreeMap;

use betterlite_config::sections::retry_policy as config;
use betterlite_router::retry_policy::backoff::BackoffSettings;
use betterlite_router::retry_policy::policy::{RetryPolicies, RetryPolicy};

mod tests;

#[derive(Debug, Clone, PartialEq)]
pub struct RetrySettings {
    pub policies: RetryPolicies,
    pub num_retries: i64,
    pub backoff: BackoffSettings,
}

#[must_use]
pub fn resolve(section: &config::Settings) -> RetrySettings {
    let mut model_group = BTreeMap::new();
    for (group, policy) in &section.model_group_retry_policy {
        model_group.insert(group.clone(), policy_from_config(policy));
    }
    RetrySettings {
        policies: RetryPolicies {
            global: section.retry_policy.as_ref().map(policy_from_config),
            model_group,
        },
        num_retries: section.num_retries,
        backoff: BackoffSettings {
            initial_retry_delay: section.initial_retry_delay,
            max_retry_delay: section.max_retry_delay,
            jitter: section.jitter,
            min_timeout: section.retry_after,
        },
    }
}

const fn policy_from_config(policy: &config::RetryPolicySettings) -> RetryPolicy {
    RetryPolicy {
        bad_request: policy.bad_request,
        authentication: policy.authentication,
        timeout: policy.timeout,
        rate_limit: policy.rate_limit,
        content_policy: policy.content_policy,
        internal_server: policy.internal_server,
        service_unavailable: policy.service_unavailable,
        not_found: policy.not_found,
        fallback: policy.fallback,
    }
}
