use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use crate::edenai_common::{
    EdenAiEnv, MissingApiKey, authorized_headers, insert_response_cost, resolve_api_base,
};
use crate::error::OpenAiError;
use crate::openai_base_config::Headers;
use crate::response_events::{
    RawResponse, ReplyBody, ResponsesReply, get_complete_url,
    transform_response as openai_transform_response,
};

mod tests;

// LiteLLM answers every should_fake_stream call with false and has no native websocket
pub const SHOULD_FAKE_STREAM: bool = false;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;

pub struct PrepareInput<'request> {
    pub headers: Headers,
    pub api_key: Option<&'request SecretString>,
    pub model: &'request str,
    pub api_base: Option<&'request str>,
    pub env: &'request EdenAiEnv,
}

pub struct Prepared {
    pub url: String,
    pub headers: Headers,
}

// the resolved Eden base goes through the OpenAI URL builder alone, so no OpenAI or global base applies
pub fn prepare(input: PrepareInput<'_>) -> Result<Prepared, MissingApiKey> {
    let base = resolve_api_base(input.api_base, input.env);
    let url = get_complete_url(Some(&base), None, None, None);
    let headers = authorized_headers(&input.headers, input.api_key, input.model, input.env)?;
    Ok(Prepared { url, headers })
}

pub struct ReplyRequest<'request, 'env> {
    pub reply: &'request RawResponse<'request>,
    pub now: i64,
    pub state: &'request mut LoggingState,
    pub env: &'request PrePostEnv<'env>,
}

// the OpenAI reply, then Eden's usage.cost goes into the hidden params when it has one
pub fn transform_response(request: ReplyRequest<'_, '_>) -> Result<ResponsesReply, OpenAiError> {
    let ReplyRequest {
        reply,
        now,
        state,
        env,
    } = request;
    let mut transformed = openai_transform_response(reply, now, state, env)?;
    if let ReplyBody::Typed(response) = &transformed.response {
        let cost = response.usage.as_ref().and_then(|usage| usage.cost);
        insert_response_cost(&mut transformed.additional_headers, cost);
    }
    Ok(transformed)
}
