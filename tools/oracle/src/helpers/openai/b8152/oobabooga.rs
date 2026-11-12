use super::invalid;
use crate::helpers::HelperError;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::local_servers::oobabooga::error::{OobaboogaError, ReplyError};
use betterlite_openai::local_servers::oobabooga::reply::{
    transform_embedding_response, transform_response,
};
use betterlite_openai::local_servers::oobabooga::{
    completion_body, completion_url, embedding_body, embeddings_url, validate_environment,
};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object as JsonObject;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestCall {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    messages: Vec<ChatMessage>,
    optional_params: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbeddingCall {
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
    input: Vec<String>,
    optional_params: JsonObject,
    reply: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCall {
    model: String,
    status: u16,
    body: String,
}

fn failure(error: &OobaboogaError) -> Value {
    json!({
        "ok": false,
        "status": error.status_code,
        "message": error.message,
    })
}

// the recorded request, with header names in the lower case httpx gives them
fn wire(url: &str, api_key: Option<&str>, body: &JsonObject) -> Value {
    let key = api_key.map(|text| SecretString::from(text.to_owned()));
    let headers: Headers = validate_environment(key.as_ref())
        .into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect();
    json!({"url": url, "headers": headers, "body": body})
}

pub(super) fn request_value(call: &RequestCall) -> Result<Value, Box<HelperError>> {
    let url = match completion_url(&call.model, call.api_base.as_deref()) {
        Ok(url) => url,
        Err(error) => return Ok(failure(&error)),
    };
    let body = completion_body(
        &call.model,
        &call.messages,
        &call.optional_params,
        &RefuseDownloads,
    )
    .map_err(|error| invalid(error.to_string()))?;
    Ok(json!({"ok": true, "request": wire(&url, call.api_key.as_deref(), &body)}))
}

pub(super) fn embedding_value(call: &EmbeddingCall) -> Result<Value, Box<HelperError>> {
    let url = match embeddings_url(&call.model, call.api_base.as_deref()) {
        Ok(url) => url,
        Err(error) => return Ok(failure(&error)),
    };
    let input = EmbeddingInput::Texts(call.input.clone());
    let body = embedding_body(&input, &call.optional_params)
        .map_err(|error| invalid(error.to_string()))?;
    match transform_embedding_response(&call.model, &call.reply) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "request": wire(&url, call.api_key.as_deref(), &body),
            "model": reply.model,
            "data": [{"embedding": reply.embedding, "index": 0, "object": "embedding"}],
            "prompt_tokens": reply.prompt_tokens,
            "total_tokens": reply.total_tokens,
        })),
        Err(ReplyError::Server(error)) => Ok(failure(&error)),
        Err(other) => Err(invalid(other.to_string())),
    }
}

// the creation time is the wall clock, so the comparison leaves it out
pub(super) fn reply_value(call: &ReplyCall) -> Result<Value, Box<HelperError>> {
    match transform_response(&call.model, call.status, &call.body, 0) {
        Ok(reply) => Ok(json!({
            "ok": true,
            "model": reply.model,
            "content": reply.content,
            "usage": reply.usage,
        })),
        Err(ReplyError::Server(error)) => Ok(failure(&error)),
        Err(other) => Err(invalid(other.to_string())),
    }
}
