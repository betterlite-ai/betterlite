use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;
use crate::finops::metering::event::{MeteringEvent, UsageTokens};

// LiteLLM reads the model, the call cost, the usage, the response id and the identities from the callback kwargs
pub(super) fn metering_event(payload: &StandardLoggingPayload) -> MeteringEvent<'_> {
    MeteringEvent {
        model: Some(payload.model.as_str()),
        response_cost: Some(payload.response_cost),
        usage: usage(payload),
        response_id: response_id(payload),
        litellm_call_id: payload.litellm_call_id.as_deref(),
        request_user: request_user(payload),
        user_api_key_user_id: payload.metadata.user_api_key_user_id.as_deref(),
        user_api_key_team_id: payload.metadata.user_api_key_team_id.as_deref(),
    }
}

// only a chat completion or an embedding response carries the usage LiteLLM copies
fn usage(payload: &StandardLoggingPayload) -> Option<UsageTokens> {
    matches!(
        payload.call_type.as_str(),
        "completion" | "acompletion" | "embedding" | "aembedding"
    )
    .then_some(UsageTokens {
        prompt_tokens: payload.prompt_tokens,
        completion_tokens: payload.completion_tokens,
        total_tokens: payload.total_tokens,
    })
}

// the payload's end_user is the key's end user id before the request's own user, so it names the request user only without one
fn request_user(payload: &StandardLoggingPayload) -> Option<&str> {
    let key_end_user = payload.metadata.user_api_key_end_user_id.as_deref();
    if key_end_user.is_some_and(|id| !id.is_empty()) {
        return None;
    }
    payload.end_user.as_deref()
}

// LiteLLM prefers the response object's own id and falls back to the litellm call id
fn response_id(payload: &StandardLoggingPayload) -> Option<&str> {
    let Json::Object(fields) = payload.response.as_ref()? else {
        return None;
    };
    match fields.get("id") {
        Some(Json::Text(id)) => Some(id.as_str()),
        _ => None,
    }
}
