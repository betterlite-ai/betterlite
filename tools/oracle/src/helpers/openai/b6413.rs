use betterlite_openai::error::ProviderError;
use betterlite_openai::hosted_chat_options::map_renaming_token_limit;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::sambanova_chat;
use betterlite_openai::sambanova_embedding::error::{SambaNovaEmbeddingError, get_error_class};
use betterlite_openai::sambanova_embedding::{self, decode_sambanova_embedding};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    chat_supported: Vec<SupportedCall>,
    #[serde(default)]
    chat_params: Vec<ParamsCall>,
    #[serde(default)]
    embedding_urls: Vec<UrlCall>,
    #[serde(default)]
    embedding_headers: Vec<HeadersCall>,
    #[serde(default)]
    embedding_requests: Vec<RequestCall>,
    #[serde(default)]
    embedding_replies: Vec<ReplyCall>,
    #[serde(default)]
    embedding_errors: Vec<ErrorCall>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportedCall {
    model: String,
    function_calling: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParamsCall {
    model: String,
    function_calling: bool,
    #[serde(default)]
    requested: IndexMap<String, Value>,
    #[serde(default)]
    existing: IndexMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadersCall {
    api_key: Option<String>,
    env_api_key: Option<String>,
    #[serde(default)]
    headers: Headers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    model: String,
    input: Value,
    #[serde(default)]
    optional: Object,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    body: String,
    status: u16,
    #[serde(default)]
    headers: Headers,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorCall {
    message: String,
    status: u16,
    #[serde(default)]
    headers: Headers,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let replies = input
            .embedding_replies
            .iter()
            .map(reply)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "chat_supported": input.chat_supported.iter().map(supported).collect::<Vec<_>>(),
            "chat_params": input.chat_params.iter().map(params).collect::<Vec<_>>(),
            "embedding_urls": input.embedding_urls.iter().map(|call| json!({
                "url": sambanova_embedding::get_complete_url(call.api_base.as_deref()).ok(),
            })).collect::<Vec<_>>(),
            "embedding_headers": input.embedding_headers.iter().map(headers).collect::<Vec<_>>(),
            "embedding_requests": input.embedding_requests.iter().map(request).collect::<Vec<_>>(),
            "embedding_replies": replies,
            "embedding_errors": input.embedding_errors.iter().map(carried).collect::<Vec<_>>(),
        }))
    }
}

fn failure(class: &str, status: Option<u16>, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn supported(call: &SupportedCall) -> Value {
    json!({
        "model": &call.model,
        "supported": sambanova_chat::supported_openai_params(call.function_calling),
    })
}

fn params(call: &ParamsCall) -> Value {
    let mut mapped = call.existing.clone();
    map_renaming_token_limit(
        &call.requested,
        &mut mapped,
        &sambanova_chat::supported_openai_params(call.function_calling),
    );
    json!({"model": &call.model, "mapped": mapped})
}

fn headers(call: &HeadersCall) -> Value {
    let explicit = call.api_key.as_deref().map(SecretString::from);
    let environment = call.env_api_key.as_deref().map(SecretString::from);
    json!({
        "headers": sambanova_embedding::validate_environment(
            &call.headers,
            explicit.as_ref(),
            environment.as_ref(),
        ),
    })
}

fn request(call: &RequestCall) -> Value {
    json!({
        "model": &call.model,
        "request": sambanova_embedding::transform_embedding_request(&call.model, &call.input, &call.optional),
    })
}

fn reply(call: &ReplyCall) -> crate::helpers::HelperResult {
    match decode_sambanova_embedding(call.body.as_bytes(), call.status, &call.headers) {
        Ok(reply) => Ok(decoded(&reply)),
        Err(SambaNovaEmbeddingError::Reply(error)) => Err(failure(
            "SambaNovaError",
            Some(error.status_code),
            &error.message,
        )),
        Err(error) => Err(failure("SambaNovaEmbeddingError", None, &error.to_string())),
    }
}

fn decoded(reply: &sambanova_embedding::SambaNovaEmbeddingResponse) -> Value {
    json!({
        "model": &reply.model,
        "object": &reply.object,
        "data": &reply.data,
        "usage": {
            "prompt_tokens": reply.usage.prompt_tokens,
            "total_tokens": reply.usage.total_tokens,
        },
    })
}

fn carried(call: &ErrorCall) -> Value {
    let error = get_error_class(&call.message, call.status, &call.headers);
    let projected = ProviderError::from(error);
    json!({
        "status_code": projected.status_code,
        "headers": &call.headers,
        "request_method": &projected.request.method,
        "request_url": &projected.request.url,
        "response_status": projected.response.status_code,
    })
}

#[path = "b6413/tests.rs"]
mod tests;
