use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::chat_config_selection::cohere::{CohereRoute, get_cohere_route};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::cohere_chat_v1::reply::{ReplyInput as V1ReplyInput, decode_cohere_chat_v1};
use betterlite_openai::cohere_chat_v1::{self, CohereV1ChatConfig};
use betterlite_openai::cohere_chat_v2;
use betterlite_openai::cohere_chat_v2::reply::{
    AssistantReply, ReplyInput as V2ReplyInput, decode_cohere_chat_v2,
};
use betterlite_openai::cohere_streams;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use uuid::Uuid;

use super::Common;
use super::error::FamilyError;
use super::request::{http_headers, url};
use super::{Call, CohereChatDecode, Decode, EmbeddingRerankPrepared, EmbeddingRerankResult};

// main.py's two Cohere chat defaults, which differ in host as well as version
const V1_DEFAULT_URL: &str = "https://api.cohere.ai/v1/chat";
const V2_DEFAULT_URL: &str = "https://api.cohere.com/v2/chat";

pub fn prepare(
    common: &Common<'_>,
    request: &ChatRequest,
    stream: bool,
) -> Result<EmbeddingRerankPrepared, FamilyError> {
    if !matches!(common.provider, ProviderId::Cohere | ProviderId::CohereChat) {
        return Err(FamilyError::Unsupported {
            provider: common.provider.to_string(),
        });
    }
    let route = get_cohere_route(common.model);
    // v2 drops every version prefix from the model it sends, and v1 keeps it
    let (model, default) = match route {
        CohereRoute::V1 => (common.model.to_owned(), V1_DEFAULT_URL),
        CohereRoute::V2 => (common.model.replace("v2/", ""), V2_DEFAULT_URL),
    };
    let base = common
        .settings
        .resolve_api_base(common.api_base)
        .unwrap_or_else(|| default.to_owned());
    let mut headers = common
        .settings
        .resolve_headers(common.headers, common.extra_headers);
    let key = common
        .settings
        .resolve_api_key(common.api_key)
        .unwrap_or_default();
    cohere_streams::validate_environment(&mut headers, &key);
    let body = body(route, &model, &base, request, stream)?;
    // LiteLLM mints the reply's id and reads the clock, so the envelope is made before the answer arrives
    let decode = Decode::CohereChat(Box::new(CohereChatDecode {
        version: route,
        model,
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs()),
        id: format!("chatcmpl-{}", Uuid::new_v4()),
    }));
    Ok(EmbeddingRerankPrepared(Call {
        url: url("cohere", &base)?,
        headers: http_headers(&headers)?,
        body: serde_json::to_vec(&Json::Object(body))?,
        timeout: common.timeout,
        decode,
    }))
}

// the reply arrives as the version's own shape, and each one decodes into the chat answer
pub(super) fn decode(
    chat: &CohereChatDecode,
    response: &UpstreamResponse,
) -> Result<EmbeddingRerankResult, FamilyError> {
    let answer = match chat.version {
        CohereRoute::V1 => {
            EmbeddingRerankResult::CohereV1Chat(Box::new(decode_cohere_chat_v1(V1ReplyInput {
                body: &response.body,
                status: response.status,
                model: &chat.model,
                created: chat.created,
                id: chat.id.clone(),
                finish_reason: None,
            })?))
        }
        CohereRoute::V2 => {
            EmbeddingRerankResult::CohereV2Chat(Box::new(decode_cohere_chat_v2(V2ReplyInput {
                body: &response.body,
                status: response.status,
                model: &chat.model,
                created: chat.created,
                id: chat.id.clone(),
                finish_reason: None,
                message: AssistantReply::default(),
                // LiteLLM mints an id for a tool call the reply left without one
                next_id: || Uuid::new_v4().to_string(),
            })?))
        }
    };
    Ok(answer)
}

fn body(
    route: CohereRoute,
    model: &str,
    base: &str,
    request: &ChatRequest,
    stream: bool,
) -> Result<Object, FamilyError> {
    let non_default = non_default_params(request, stream)?;
    match route {
        CohereRoute::V1 => {
            let mapped = cohere_chat_v1::map_openai_params(&non_default, Object::new());
            let config = CohereV1ChatConfig::with_settings(Object::new())?;
            let value = serde_json::to_value(&request.messages)?;
            let messages: Vec<Message> = serde_json::from_value(value)?;
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
                messages: &request.messages,
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
    let mut fields: Object = serde_json::from_value(serde_json::to_value(request)?)?;
    fields.shift_remove("model");
    fields.shift_remove("messages");
    fields.shift_remove("stream");
    fields.retain(|_, value| !matches!(value, Json::Null));
    if stream {
        fields.insert(String::from("stream"), Json::Bool(true));
    }
    Ok(fields)
}
