use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_content::RefuseDownloads;
use betterlite_openai::cometapi_wire::chat::{
    ChatError, ChatTransform, complete_url, transform_request,
};
use betterlite_openai::cometapi_wire::chunk::chunk_parser;
use betterlite_openai::cometapi_wire::embed::{decode_comet_embedding_reply, validate_environment};
use betterlite_prompts::arguments::json::Object as JsonObject;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
mod tests;
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Request { requests: Vec<RequestCall> },
    Chunks { chunks: Vec<JsonObject> },
    EmbeddingHeaders { calls: Vec<HeaderCall> },
    EmbeddingReplies { replies: Vec<ReplyCall> },
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestCall {
    api_base: Option<String>,
    model: String,
    messages: Vec<ChatMessage>,
    optional_params: JsonObject,
    env: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderCall {
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    env: BTreeMap<String, String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    body: String,
    status: u16,
    headers: BTreeMap<String, String>,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Request { requests } => {
                Ok(Value::Array(requests.iter().map(request_value).collect()))
            }
            Input::Chunks { chunks } => Ok(Value::Array(chunks.iter().map(chunk_value).collect())),
            Input::EmbeddingHeaders { calls } => Ok(Value::Array(
                calls
                    .iter()
                    .map(header_value)
                    .collect::<Result<Vec<Value>, Box<HelperError>>>()?,
            )),
            Input::EmbeddingReplies { replies } => {
                Ok(Value::Array(replies.iter().map(reply_value).collect()))
            }
        }
    }
}
fn request_value(call: &RequestCall) -> Value {
    let transform = ChatTransform {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        environment: &call.env,
        downloads: &RefuseDownloads,
    };
    let url = complete_url(call.api_base.as_deref());
    match transform_request(&transform) {
        Ok(body) => json!({"url": url, "ok": true, "value": body}),
        Err(ChatError::ExtraBody | ChatError::Content(_)) => json!({"url": url, "ok": false}),
    }
}
fn chunk_value(chunk: &JsonObject) -> Value {
    match chunk_parser(chunk) {
        Ok(parsed) => json!({"ok": true, "value": parsed}),
        Err(error) => json!({
            "ok": false,
            "class": "CometAPIException",
            "status": error.status_code,
        }),
    }
}
fn header_value(call: &HeaderCall) -> Result<Value, Box<HelperError>> {
    let settings = ConfigLoader {
        lookup: call.env.clone(),
    }
    .load("")
    .map(|config| config.sections.cometapi_embedding_credentials)
    .map_err(|error| invalid(error.to_string()))?;
    let key = call.api_key.clone().map(SecretString::from);
    Ok(json!({"headers": validate_environment(&call.headers, key.as_ref(), &settings)}))
}
fn reply_value(call: &ReplyCall) -> Value {
    match decode_comet_embedding_reply(call.body.as_bytes(), call.status, &call.headers) {
        Ok(reply) => json!({"ok": true, "value": reply}),
        Err(error) => json!({
            "ok": false,
            "class": "CometAPIException",
            "status": error.status_code,
        }),
    }
}
fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
