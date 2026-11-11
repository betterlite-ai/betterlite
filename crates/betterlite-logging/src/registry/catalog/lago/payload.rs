use crate::fields::payload::StandardLoggingPayload;
use crate::finops::metering::event::{MeteringEvent, UsageTokens};

// LiteLLM reads the model, the call cost, the usage and the key's identities from the callback kwargs
pub(super) fn metering_event(payload: &StandardLoggingPayload) -> MeteringEvent<'_> {
    MeteringEvent {
        model: Some(payload.model.as_str()),
        response_cost: Some(payload.response_cost),
        usage: Some(UsageTokens {
            prompt_tokens: payload.prompt_tokens,
            completion_tokens: payload.completion_tokens,
            total_tokens: payload.total_tokens,
        }),
        response_id: None,
        litellm_call_id: payload.litellm_call_id.as_deref(),
        request_user: payload.end_user.as_deref(),
        user_api_key_user_id: payload.metadata.user_api_key_user_id.as_deref(),
        user_api_key_team_id: payload.metadata.user_api_key_team_id.as_deref(),
    }
}
