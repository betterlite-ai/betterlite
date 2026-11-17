use betterlite_core::api::responses_usage::Usage;
use chrono::{DateTime, Utc};

use super::RouterObservations;
use super::counter::CounterSink;
use super::keys::{ROUTER_COUNTER_TTL, outcome_keys};
use super::usage::{DeploymentUsage, UsageIncrement};

// the request owns one of these: the response hook records the tokens it counted, a cancelled request drops it counted once
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservationMetadata {
    pub counted_tokens: Option<i64>,
}

// metadata and litellm_params.model_info as the success callbacks read them
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestFacts<'a> {
    pub model_group: Option<&'a str>,
    pub deployment_name: &'a str,
    pub deployment_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuccessEvent<'a> {
    pub call_type: Option<&'a str>,
    // standard_logging_object.total_tokens, with None standing for a missing standard_logging_object
    pub total_tokens: Option<i64>,
    pub request: RequestFacts<'a>,
    pub deployment: Option<&'a DeploymentUsage>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResponseEvent<'a> {
    pub usage: &'a Usage,
    pub request: RequestFacts<'a>,
    pub deployment: Option<&'a DeploymentUsage>,
}

// the wrappers that hand a whole session's usage to the per-turn callbacks must not count it again
fn skips_usage(call_type: Option<&str>) -> bool {
    call_type.is_some_and(|kind| {
        matches!(
            kind,
            "_aresponses_websocket" | "_arealtime" | "retrieve_batch" | "aretrieve_batch"
        )
    })
}

impl RouterObservations<'_> {
    pub fn on_success(
        &self,
        event: &SuccessEvent<'_>,
        metadata: &ObservationMetadata,
        now: DateTime<Utc>,
    ) -> Option<String> {
        if skips_usage(event.call_type) {
            return None;
        }
        let total_tokens = event.total_tokens?;
        let deployment = event.deployment?;
        let deployment_id = event.request.deployment_id?;
        if event.request.model_group.is_none() {
            return None;
        }
        let keys = outcome_keys(deployment_id);
        // LiteLLM only writes a debug log when the success counter cannot be incremented
        if self
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
        self.increment_deployment_usage(&increment, Some(deployment), now)
            .ok()
            .flatten()
    }

    pub fn count_response_usage(
        &self,
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
        match self.increment_deployment_usage(&increment, event.deployment, now) {
            Ok(key) => key,
            // the marker goes only once the usage write landed, so a retried response counts its tokens once
            Err(_) => {
                metadata.counted_tokens = None;
                None
            }
        }
    }
}
