use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::router_observations::counter::Increment;

use super::keys::window_keys;

// batch_utils.BATCH_RETRIEVE_CALL_TYPES: a batch retrieve reports the job's tokens again on every poll
const BATCH_RETRIEVE_CALL_TYPES: [&str; 2] = ["aretrieve_batch", "retrieve_batch"];

// StandardLoggingPayload.hidden_params: the provider model LiteLLM sent, the second half of the tpm key
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiddenParams<'a> {
    pub litellm_model_name: Option<&'a str>,
}

// the StandardLoggingPayload fields both success callbacks read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandardLoggingObject<'a> {
    pub model_group: Option<&'a str>,
    pub hidden_params: HiddenParams<'a>,
    pub model_id: Option<&'a str>,
    pub total_tokens: i64,
}

// the logging kwargs a success callback receives: the call type and the payload the proxy logged
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuccessEvent<'a> {
    pub call_type: Option<&'a str>,
    pub standard_logging_object: Option<StandardLoggingObject<'a>>,
}

// both callbacks skip the same calls and write the same key; only the cache primitive differs
pub(super) fn success_increment(
    event: &SuccessEvent<'_>,
    now: DateTime<Utc>,
    ttl: Duration,
) -> Option<Increment> {
    if event
        .call_type
        .is_some_and(|call_type| BATCH_RETRIEVE_CALL_TYPES.contains(&call_type))
    {
        return None;
    }
    let Some(payload) = event.standard_logging_object else {
        tracing::warn!("a success callback arrived without a standard logging object");
        return None;
    };
    let (Some(_), Some(model_id), Some(model)) = (
        payload.model_group,
        payload.model_id,
        payload.hidden_params.litellm_model_name,
    ) else {
        return None;
    };
    Some(Increment {
        key: window_keys(model_id, model, now).tpm,
        value: payload.total_tokens,
        ttl,
    })
}
