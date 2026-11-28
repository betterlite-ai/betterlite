use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::response::{RawResponse, ResponseRequest};
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_models::decisions::Json;

use crate::vertex_claude_chat::response::{
    TransformCall, transform_response as transform_vertex_response,
};

use super::PartnerResponseCall;
use super::error::PartnerResponseError;

// B1333 fills a fresh envelope from the body and names the requested model; the dump is the answer.
pub(super) fn transform_response<Counter>(
    call: PartnerResponseCall<'_, Counter>,
) -> Result<Json, PartnerResponseError<Counter::Failure>>
where
    Counter: ReasoningTokenCounter,
{
    let PartnerResponseCall { input, counter } = call;
    let mut envelope = ResponseEnvelope::new(input.starting_id.to_owned());
    let raw = RawResponse {
        status: input.status,
        headers: input.headers,
        body: input.body,
    };
    let request = ResponseRequest {
        messages: input.messages,
        optional_params: input.optional_params,
        litellm_params: input.litellm_params,
        request_data: input.request_data,
        json_mode: input.json_mode,
        now: input.now,
        disable_add_prefix_to_prompt: input.disable_add_prefix_to_prompt,
    };
    transform_vertex_response(TransformCall {
        model: input.model,
        envelope: &mut envelope,
        raw: &raw,
        request: &request,
        state: input.state,
        env: input.env,
        counter,
    })?;
    Ok(serde_json::from_value(serde_json::to_value(&envelope)?)?)
}
