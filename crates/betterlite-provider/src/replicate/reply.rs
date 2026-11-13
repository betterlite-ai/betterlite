use std::str::from_utf8;

use async_trait::async_trait;
use betterlite_logging::analytics::python;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message;
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;

use super::error::{ReplicateError, ReplicateFailure};
use super::poll::OutputPolicy;
use super::request::ReplicateBody;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicateUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicateReply {
    pub model: String,
    pub created: Option<i64>,
    pub text: String,
    pub usage: ReplicateUsage,
}

#[async_trait]
pub trait TokenCounter: Send + Sync {
    async fn count_messages(&self, model: &str, messages: &[Message]) -> Result<i64, CountError>;
    async fn count_response_tokens(&self, model: &str, text: &str) -> Result<i64, CountError>;
}

pub struct PostCall<'request> {
    pub input: &'request [Message],
    pub api_key: Option<&'request str>,
    pub original_response: &'request str,
    pub request_data: &'request ReplicateBody,
}

pub trait PostCallLogger: Send + Sync {
    fn post_call(&self, call: PostCall<'_>);
}

pub struct ReplyInput<'request> {
    pub model: &'request str,
    pub body: &'request [u8],
    pub headers: &'request HeaderMap,
    pub messages: &'request [Message],
    pub request_data: &'request ReplicateBody,
    pub api_key: Option<&'request str>,
}

// Python's format() of a parsed JSON value: a string stays itself, None is None, a list or object is its repr
pub(crate) fn json_display(value: &Json) -> Result<String, ReplicateFailure> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Null => Ok(String::from("None")),
        Json::Bool(true) => Ok(String::from("True")),
        Json::Bool(false) => Ok(String::from("False")),
        Json::Number(number) => Ok(number.to_string()),
        Json::List(_) | Json::Object(_) => {
            python::repr(value).map_err(|source| ReplicateFailure::Format { source })
        }
    }
}

fn unparsable_output(value: &Json, headers: &HeaderMap) -> ReplicateFailure {
    let rendered = match json_display(value) {
        Ok(rendered) => rendered,
        Err(failure) => return failure,
    };
    ReplicateFailure::Api(Box::new(ReplicateError {
        status_code: 422,
        message: format!("Unable to parse response. Got={rendered}"),
        headers: Some(headers.clone()),
    }))
}

fn join_list(
    items: &[Json],
    output: &Json,
    headers: &HeaderMap,
) -> Result<String, ReplicateFailure> {
    let mut joined = String::new();
    for item in items {
        let Json::Text(part) = item else {
            return Err(unparsable_output(output, headers));
        };
        joined.push_str(part);
    }
    Ok(joined)
}

pub(crate) fn join_output(
    output: &Json,
    policy: OutputPolicy,
    headers: &HeaderMap,
) -> Result<String, ReplicateFailure> {
    if let Json::List(items) = output {
        return join_list(items, output, headers);
    }
    if let Json::Text(text) = output {
        return Ok(text.clone());
    }
    match policy {
        OutputPolicy::Strict => Err(unparsable_output(output, headers)),
        OutputPolicy::Lenient => json_display(output),
    }
}

pub fn get_prediction_url(body: &[u8], headers: &HeaderMap) -> Result<String, ReplicateFailure> {
    let value: Json =
        serde_json::from_slice(body).map_err(|source| ReplicateFailure::Decode { source })?;
    let Some(object) = value.as_object() else {
        return Err(ReplicateFailure::ReplyNotObject);
    };
    match object
        .get("urls")
        .and_then(Json::as_object)
        .and_then(|urls| urls.get("get"))
    {
        Some(Json::Text(url)) => Ok(url.clone()),
        Some(Json::Null) | None => {
            let rendered =
                python::repr(&value).map_err(|source| ReplicateFailure::Format { source })?;
            Err(ReplicateFailure::Api(Box::new(ReplicateError {
                status_code: 400,
                message: format!("LiteLLM Error - prediction url is None - {rendered}"),
                headers: Some(headers.clone()),
            })))
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ReplicateFailure::PredictionUrlType)
        }
    }
}

fn log_reply(reply: &ReplyInput<'_>, logger: &dyn PostCallLogger) -> Result<(), ReplicateFailure> {
    let raw = from_utf8(reply.body).map_err(|source| ReplicateFailure::BodyNotUtf8 { source })?;
    logger.post_call(PostCall {
        input: reply.messages,
        api_key: reply.api_key,
        original_response: raw,
        request_data: reply.request_data,
    });
    Ok(())
}

fn prediction_text(object: &Object, headers: &HeaderMap) -> Result<String, ReplicateFailure> {
    let Some(output) = object.get("output") else {
        return Ok(String::from(" "));
    };
    let joined = join_output(output, OutputPolicy::Strict, headers)?;
    if joined.is_empty() {
        return Ok(String::from(" "));
    }
    Ok(joined)
}

async fn count_usage(
    counter: &dyn TokenCounter,
    model: &str,
    messages: &[Message],
    text: &str,
) -> Result<ReplicateUsage, ReplicateFailure> {
    let prompt_tokens = counter
        .count_messages(model, messages)
        .await
        .map_err(|source| ReplicateFailure::TokenCount { source })?;
    let completion_tokens = counter
        .count_response_tokens(model, text)
        .await
        .map_err(|source| ReplicateFailure::TokenCount { source })?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(ReplicateFailure::UsageOverflow)?;
    Ok(ReplicateUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
    })
}

pub async fn transform_response(
    reply: &ReplyInput<'_>,
    counter: &dyn TokenCounter,
    logger: &dyn PostCallLogger,
) -> Result<ReplicateReply, ReplicateFailure> {
    log_reply(reply, logger)?;

    let value: Json =
        serde_json::from_slice(reply.body).map_err(|source| ReplicateFailure::Decode { source })?;
    let Some(object) = value.as_object() else {
        return Err(ReplicateFailure::ReplyNotObject);
    };
    let succeeded =
        matches!(object.get("status"), Some(Json::Text(status)) if status == "succeeded");
    if !succeeded {
        let rendered =
            python::repr(&value).map_err(|source| ReplicateFailure::Format { source })?;
        return Err(ReplicateFailure::Api(Box::new(ReplicateError {
            status_code: 422,
            message: format!("LiteLLM Error - prediction not succeeded - {rendered}"),
            headers: Some(reply.headers.clone()),
        })));
    }

    let text = prediction_text(object, reply.headers)?;
    let usage = count_usage(counter, reply.model, reply.messages, &text).await?;
    Ok(ReplicateReply {
        model: format!("replicate/{}", reply.model),
        created: None,
        text,
        usage,
    })
}

mod tests;
