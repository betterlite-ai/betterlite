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

// why a call stopped before it had an answer to record
pub(super) enum Stop {
    Server(Box<OobaboogaError>),
    Reply(Box<ReplyError>),
    Input(String),
}

pub(super) fn finish(result: Result<Value, Stop>) -> Result<Value, Box<HelperError>> {
    match result {
        Ok(value) => Ok(value),
        Err(Stop::Server(error)) => Ok(failure(&error)),
        Err(Stop::Reply(reply)) => match *reply {
            ReplyError::Server(error) => Ok(failure(&error)),
            other @ (ReplyError::Shape { .. } | ReplyError::Missing { .. }) => {
                Err(invalid(other.to_string()))
            }
        },
        Err(Stop::Input(message)) => Err(invalid(message)),
    }
}

// the call the helper was asked to run
pub(super) enum Call<'call> {
    Request(&'call RequestCall),
    Embedding(&'call EmbeddingCall),
    Reply(&'call ReplyCall),
}

// the creation time is the wall clock, so the comparison leaves it out
pub(super) fn record(which: &Call<'_>) -> Result<Value, Stop> {
    match which {
        Call::Request(call) => {
            let url = completion_url(&call.model, call.api_base.as_deref())
                .map_err(|error| Stop::Server(Box::new(error)))?;
            let messages = &call.messages;
            let params = &call.optional_params;
            let body = completion_body(&call.model, messages, params, &RefuseDownloads)
                .map_err(|error| Stop::Input(error.to_string()))?;
            Ok(json!({"ok": true, "request": wire(&url, call.api_key.as_deref(), &body)}))
        }
        Call::Embedding(call) => {
            let url = embeddings_url(&call.model, call.api_base.as_deref())
                .map_err(|error| Stop::Server(Box::new(error)))?;
            let input = EmbeddingInput::Texts(call.input.clone());
            let body = embedding_body(&input, &call.optional_params)
                .map_err(|error| Stop::Input(error.to_string()))?;
            let reply = transform_embedding_response(&call.model, &call.reply)
                .map_err(|error| Stop::Reply(Box::new(error)))?;
            Ok(json!({
                "ok": true,
                "request": wire(&url, call.api_key.as_deref(), &body),
                "model": reply.model,
                "data": [{"embedding": reply.embedding, "index": 0, "object": "embedding"}],
                "prompt_tokens": reply.prompt_tokens,
                "total_tokens": reply.total_tokens,
            }))
        }
        Call::Reply(call) => {
            let chat = transform_response(&call.model, call.status, &call.body, 0)
                .map_err(|error| Stop::Reply(Box::new(error)))?;
            Ok(json!({
                "ok": true,
                "model": chat.model,
                "content": chat.content,
                "usage": chat.usage,
            }))
        }
    }
}
