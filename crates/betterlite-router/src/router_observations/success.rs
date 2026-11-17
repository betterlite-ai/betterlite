use betterlite_core::api::responses_usage::Usage;
use chrono::{DateTime, Utc};

use super::RouterObservations;
use super::keys::{ROUTER_COUNTER_TTL, outcome_keys};
use super::usage::{DeploymentUsage, UsageIncrement, increment_deployment_usage};

// the request owns one of these: the response hook records the tokens it counted, a cancelled request drops it counted once
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservationMetadata {
    pub counted_tokens: Option<i64>,
}

// metadata and litellm_params.model_info as the success callbacks read them
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestFacts<'request> {
    pub model_group: Option<&'request str>,
    pub deployment_name: &'request str,
    pub deployment_id: Option<&'request str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuccessEvent<'event> {
    pub call_type: Option<&'event str>,
    // standard_logging_object.total_tokens, with None standing for a missing standard_logging_object
    pub total_tokens: Option<i64>,
    pub request: RequestFacts<'event>,
    pub deployment: Option<&'event DeploymentUsage>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseEvent<'response> {
    pub usage: &'response Usage,
    pub request: RequestFacts<'response>,
    pub deployment: Option<&'response DeploymentUsage>,
}

// the wrappers that hand a whole session's usage to the per-turn callbacks must not count it again
const SKIPPED_CALL_TYPES: [&str; 4] = [
    "_aresponses_websocket",
    "_arealtime",
    "retrieve_batch",
    "aretrieve_batch",
];

#[must_use]
pub fn on_success(
    observations: &RouterObservations<'_>,
    event: &SuccessEvent<'_>,
    metadata: &ObservationMetadata,
    now: DateTime<Utc>,
) -> Option<String> {
    if event
        .call_type
        .is_some_and(|kind| SKIPPED_CALL_TYPES.contains(&kind))
    {
        return None;
    }
    let total_tokens = event.total_tokens?;
    let deployment = event.deployment?;
    let deployment_id = event.request.deployment_id?;
    event.request.model_group?;
    let keys = outcome_keys(deployment_id);
    // LiteLLM only writes a debug log when the success counter cannot be incremented
    if observations
        .counters
        .increment(&keys.successes, 1, ROUTER_COUNTER_TTL)
        .is_err()
    {
        return None;
    }
    let counted_tokens = metadata.counted_tokens;
    let tokens = counted_tokens.map_or(total_tokens, |counted| {
        total_tokens.saturating_sub(counted).max(0)
    });
    let rpm_increment = i64::from(counted_tokens.is_none());
    let increment = UsageIncrement {
        deployment_id,
        deployment_name: event.request.deployment_name,
        total_tokens: tokens,
        rpm_increment,
    };
    increment_deployment_usage(observations, &increment, Some(deployment), now)
        .ok()
        .flatten()
}

pub fn count_response_usage(
    observations: &RouterObservations<'_>,
    event: &ResponseEvent<'_>,
    metadata: &mut ObservationMetadata,
    now: DateTime<Utc>,
) -> Option<String> {
    let (Some(_model_group), Some(deployment_id)) =
        (event.request.model_group, event.request.deployment_id)
    else {
        return None;
    };
    let total_tokens = event.usage.total_tokens;
    metadata.counted_tokens = Some(total_tokens);
    let increment = UsageIncrement {
        deployment_id,
        deployment_name: event.request.deployment_name,
        total_tokens,
        rpm_increment: 1,
    };
    if let Ok(key) = increment_deployment_usage(observations, &increment, event.deployment, now) {
        key
    } else {
        // the marker goes only once the usage write landed, so a retried response counts its tokens once
        metadata.counted_tokens = None;
        None
    }
}
