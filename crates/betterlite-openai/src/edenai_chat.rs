use std::error::Error;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{HeaderSource, process_response_headers};
use betterlite_models::decisions::{HiddenParams, Json as HiddenJson};
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

use crate::chat::ChatMessage;
use crate::chat::response::ChatResponse;
use crate::chat_choices::stream::{self, StreamError};
use crate::chat_choices::wire::StreamReply;
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::compatible_wire::{
    CompatibleWireError, LogInput, ReplyOptions, transform_response as openai_transform_response,
};
use crate::edenai_common::{
    EdenAiEnv, EdenAiError, PROVIDER, catalog_reply_error, cost_number, endpoint_url,
    parse_model_catalog, reported_cost, reported_cost_from_bytes, resolve_api_base,
    resolve_api_key, set_response_cost,
};
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

mod tests;

const ADDITIONAL_HEADERS: &str = "additional_headers";
const INCLUDE_USAGE: &str = "include_usage";
const REASONING_EFFORT: &str = "reasoning_effort";
const STREAM_OPTIONS: &str = "stream_options";

// LiteLLM asks litellm.supports_reasoning for provider edenai; the answer arrives through this seam
pub trait ReasoningLookup {
    type Error: Error;

    fn supports_reasoning(&self, model: &str) -> Result<bool, Self::Error>;
}

#[derive(Debug, Error)]
pub enum EdenAiChatError {
    #[error(transparent)]
    Content(#[from] ChatContentError),
    #[error("stream_options must be a JSON object")]
    StreamOptions,
}

impl EdenAiChatError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Content(error) => error.status_code(),
            Self::StreamOptions => 400,
        }
    }
}

pub struct TransformInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// the parent list plus reasoning_effort when the capability says the model reasons; a failed lookup is no parameter
pub fn supported_params<Lookup>(
    lookup: &Lookup,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str>
where
    Lookup: ReasoningLookup,
{
    let mut params = OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
    .supported_params(model, catalog);
    match lookup.supports_reasoning(model) {
        Ok(true) => params.push(REASONING_EFFORT),
        Ok(false) => {}
        Err(error) => {
            tracing::debug!(%error, model, "error checking if the model supports reasoning");
        }
    }
    params
}

// LiteLLM's static get_api_key: the explicit key, then EDENAI_API_KEY
#[must_use]
pub fn get_api_key(api_key: Option<&SecretString>, env: &EdenAiEnv) -> Option<SecretString> {
    resolve_api_key(api_key, env)
}

// LiteLLM's static get_api_base: the explicit base, then EDENAI_API_BASE, then the default
#[must_use]
pub fn get_api_base(api_base: Option<&str>, env: &EdenAiEnv) -> String {
    resolve_api_base(api_base, env)
}

// the OpenAI request with include_usage forced into stream_options, and only on a stream
pub fn transform_request(
    input: &TransformInput<'_>,
    mode: Mode,
) -> Result<Object, EdenAiChatError> {
    let content = ChatContent {
        model: input.model,
        messages: input.messages,
        optional_params: input.optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment: input.environment,
        downloads: input.downloads,
    };
    let mut body = content.transform(mode)?.to_body()?;
    if body.get("stream") == Some(&Json::Bool(true)) {
        let options = stream_options_with_usage(body.get(STREAM_OPTIONS))?;
        body.insert(STREAM_OPTIONS.to_owned(), options);
    }
    Ok(body)
}

// the existing options, where null counts as absent, plus include_usage; a non-mapping is a typed error
fn stream_options_with_usage(current: Option<&Json>) -> Result<Json, EdenAiChatError> {
    let mut options = match current {
        None | Some(Json::Null) => Object::new(),
        Some(Json::Object(fields)) => fields.clone(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(EdenAiChatError::StreamOptions);
        }
    };
    options.insert(INCLUDE_USAGE.to_owned(), Json::Bool(true));
    Ok(Json::Object(options))
}

pub struct ReplyRequest<'request, 'call, 'env> {
    pub body: &'request [u8],
    pub headers: &'request IndexMap<String, String>,
    pub state: &'request mut LoggingState,
    pub env: &'request PrePostEnv<'env>,
    pub input: LogInput,
    pub options: ReplyOptions<'call>,
}

pub struct ChatReply {
    pub response: ChatResponse,
    pub hidden_model: Option<String>,
    pub hidden_params: HiddenParams,
}

// the OpenAI reply transform, then the cost Eden reports at the top level goes into the hidden params
pub fn transform_response(
    request: ReplyRequest<'_, '_, '_>,
) -> Result<ChatReply, CompatibleWireError> {
    let ReplyRequest {
        body,
        headers,
        state,
        env,
        input,
        options,
    } = request;
    let reply = openai_transform_response(body, &options, state, env, input)?;
    let mut hidden_params = HiddenParams::new();
    hidden_params.insert(
        ADDITIONAL_HEADERS.to_owned(),
        HiddenJson::Object(process_response_headers(HeaderSource::Upstream(headers))),
    );
    set_response_cost(&mut hidden_params, reported_cost_from_bytes(body));
    Ok(ChatReply {
        response: reply.response,
        hidden_model: reply.hidden_model,
        hidden_params,
    })
}

// the OpenAI chunk parser, then the chunk's cost lands on the usage that chunk already carries
pub fn chunk_parser(
    chunk: &Value,
    identifier: fn() -> Uuid,
    clock: fn() -> i64,
) -> Result<StreamReply, StreamError> {
    let mut parsed = stream::chunk_parser(chunk, identifier, clock)?;
    let cost = reported_cost(chunk);
    if let (Some(cost), Some(Value::Object(usage))) = (cost, parsed.usage.as_mut())
        && let Some(number) = cost_number(cost)
    {
        usage.insert(String::from("cost"), Value::Number(number));
    }
    Ok(parsed)
}

pub struct CatalogReply<'reply> {
    pub status: u16,
    pub headers: &'reply Headers,
    pub text: &'reply str,
}

// LiteLLM's get_models reply: a failed status keeps its text and headers, and a bad body is the typed error
pub fn get_models(reply: &CatalogReply<'_>) -> Result<Vec<String>, EdenAiError> {
    if !(200..300).contains(&reply.status) {
        return Err(EdenAiError {
            status_code: reply.status,
            message: reply.text.to_owned(),
            headers: reply.headers.clone(),
        });
    }
    let ids = parse_model_catalog(reply.text.as_bytes())
        .map_err(|error| catalog_reply_error(error, reply.headers))?;
    Ok(ids
        .into_iter()
        .map(|id| format!("{PROVIDER}/{id}"))
        .collect())
}

// the GET itself is the caller's: LiteLLM sends it without an Authorization header
#[must_use]
pub fn models_url(api_base: Option<&str>, env: &EdenAiEnv) -> String {
    endpoint_url(api_base, "models", env)
}
