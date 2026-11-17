use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::embedding_provider_settings::Settings;
use betterlite_context::chat_config_selection::cohere::{CohereRoute, get_cohere_route};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::cohere_chat_v1::{self, CohereV1ChatConfig};
use betterlite_openai::cohere_chat_v2;
use betterlite_openai::cohere_streams;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;
use uuid::Uuid;

use super::error::FamilyError;
use super::request::{encode, http_headers, plain, url};
use super::{Call, CohereChatDecode, Decode, EmbeddingRerankPrepared};

// main.py's two Cohere chat defaults, which differ in host as well as version
const V1_DEFAULT_URL: &str = "https://api.cohere.ai/v1/chat";
const V2_DEFAULT_URL: &str = "https://api.cohere.com/v2/chat";

// the fields LiteLLM reads itself, so none of them is an optional provider parameter
const REMOVED_FIELDS: [&str; 3] = ["model", "messages", "stream"];

pub struct ChatCallInput<'request> {
    pub provider: ProviderId,
    pub model: &'request str,
    pub request: &'request ChatRequest,
    pub stream: bool,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub headers: Option<&'request Headers>,
    pub extra_headers: &'request Headers,
    pub settings: &'request Settings,
    pub timeout: Duration,
}

pub fn prepare(input: &ChatCallInput<'_>) -> Result<EmbeddingRerankPrepared, FamilyError> {
    if !matches!(input.provider, ProviderId::Cohere | ProviderId::CohereChat) {
        return Err(FamilyError::Unsupported {
            provider: input.provider.to_string(),
        });
    }
    let route = get_cohere_route(input.model);
    // v2 drops every version prefix from the model it sends, and v1 keeps it
    let model = match route {
        CohereRoute::V1 => input.model.to_owned(),
        CohereRoute::V2 => input.model.replace("v2/", ""),
    };
    let default = match route {
        CohereRoute::V1 => V1_DEFAULT_URL,
        CohereRoute::V2 => V2_DEFAULT_URL,
    };
    let base = input
        .settings
        .resolve_api_base(input.api_base)
        .unwrap_or_else(|| default.to_owned());
    let mut headers = input
        .settings
        .resolve_headers(input.headers, input.extra_headers);
    let key = input
        .settings
        .resolve_api_key(input.api_key)
        .unwrap_or_else(|| SecretString::from(String::new()));
    cohere_streams::validate_environment(&mut headers, &key);
    let body = body(route, &model, &base, input)?;
    Ok(EmbeddingRerankPrepared(Call {
        url: url("cohere", &base)?,
        headers: http_headers(&headers)?,
        body: encode(&Json::Object(body))?,
        timeout: input.timeout,
        decode: Decode::CohereChat(Box::new(chat_decode(route, model))),
    }))
}

// LiteLLM mints the reply's id and reads the clock, so the envelope is made before the answer arrives
fn chat_decode(route: CohereRoute, model: String) -> CohereChatDecode {
    CohereChatDecode {
        version: route,
        model,
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
        id: format!("chatcmpl-{}", Uuid::new_v4()),
    }
}

fn body(
    route: CohereRoute,
    model: &str,
    base: &str,
    input: &ChatCallInput<'_>,
) -> Result<Object, FamilyError> {
    let non_default = non_default_params(input.request, input.stream)?;
    match route {
        CohereRoute::V1 => {
            let mapped = cohere_chat_v1::map_openai_params(&non_default, Object::new());
            let config = CohereV1ChatConfig::with_settings(Object::new())?;
            let value = serde_json::to_value(&input.request.messages)
                .map_err(|source| FamilyError::Encode { source })?;
            let messages: Vec<Message> =
                serde_json::from_value(value).map_err(|source| FamilyError::Encode { source })?;
            let options = cohere_chat_v1::request_options(&cohere_chat_v1::RequestInput {
                config: &config,
                messages: &messages,
                optional_params: &mapped,
            })?;
            Ok(options)
        }
        CohereRoute::V2 => {
            let mapped = cohere_chat_v2::map_openai_params(&non_default, Object::new());
            let environment: BTreeMap<String, String> = BTreeMap::new();
            let complete = cohere_chat_v2::complete_url(Some(base))?;
            let body = cohere_chat_v2::request_body(&cohere_chat_v2::RequestInput {
                model,
                messages: &input.request.messages,
                optional_params: &mapped,
                api_base: Some(complete.as_str()),
                environment: &environment,
                downloads: &RefuseDownloads,
            })?;
            Ok(body)
        }
    }
}

// LiteLLM's get_optional_params receives only the parameters the caller set, so a null field is absent
fn non_default_params(request: &ChatRequest, stream: bool) -> Result<Object, FamilyError> {
    let mut fields: Object = plain(request)?;
    for name in REMOVED_FIELDS {
        fields.shift_remove(name);
    }
    fields.retain(|_, value| !matches!(value, Json::Null));
    if stream {
        fields.insert(String::from("stream"), Json::Bool(true));
    }
    Ok(fields)
}
