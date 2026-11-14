use betterlite_anthropic::message_replay::cache_control::normalize_cache_control;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock::projection::project_message;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::media::parts::INLINE_EVERY_REMOTE_URL;
use betterlite_prompts::message_content::Message;
use serde_json::Number;
use thiserror::Error;

use super::config::is_claude_model;
use super::messages::MessageError;
use super::messages::tools::{ToolError, transform_tool_choice, transform_tools};

const MODEL: &str = "model";
const MESSAGES: &str = "messages";
const STREAM: &str = "stream";
const EXTRA_BODY: &str = "extra_body";
const SYSTEM: &str = "system";
const TOOLS: &str = "tools";
const TOOL_CHOICE: &str = "tool_choice";
const MAX_TOKENS: &str = "max_tokens";
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const DEFAULT_MAX_TOKENS: i64 = 4096;

#[derive(Debug, Error)]
pub enum RequestError {
    #[error(transparent)]
    Messages(#[from] MessageError),
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error(transparent)]
    Media(#[from] MediaError),
    #[error("extra_body must be an object")]
    ExtraBody,
    #[error("{field} must be a list")]
    List { field: &'static str },
}

// The values both body builders share: the model as sent, the turns, the stream flag and extra_body
struct RequestParts<'call> {
    model: &'call str,
    messages: &'call [Message],
    stream: Json,
    extra_body: Object,
}

pub async fn transform_request<Source>(
    model: &str,
    messages: &[Message],
    optional_params: Object,
    fetcher: &Source,
) -> Result<Object, RequestError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let mut params = optional_params;
    let stream = match params.shift_remove(STREAM) {
        Some(value) if value.is_truthy() => value,
        Some(_) | None => Json::Bool(false),
    };
    let extra_body = match params.shift_remove(EXTRA_BODY) {
        None => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(RequestError::ExtraBody);
        }
    };
    let parts = RequestParts {
        model,
        messages,
        stream,
        extra_body,
    };
    if is_claude_model(model) {
        claude_body(&parts, params, fetcher).await
    } else {
        Ok(openai_body(&parts, params))
    }
}

// LiteLLM inlines remote media only for the Claude dialect; the OpenAI body keeps the URLs as sent
pub async fn async_transform_request<Source>(
    model: &str,
    messages: &[Message],
    optional_params: Object,
    fetcher: &Source,
) -> Result<Object, RequestError>
where
    Source: RemoteMediaFetcher + Sync,
{
    if !is_claude_model(model) {
        return transform_request(model, messages, optional_params, fetcher).await;
    }
    let inlined = async_inline_remote_media(messages, &INLINE_EVERY_REMOTE_URL, fetcher).await?;
    transform_request(model, &inlined, optional_params, fetcher).await
}

fn openai_body(parts: &RequestParts<'_>, mut params: Object) -> Object {
    let max_tokens = params.shift_remove(MAX_TOKENS);
    let max_completion_tokens = params.shift_remove(MAX_COMPLETION_TOKENS);
    let resolved = resolved_max(max_completion_tokens, max_tokens);
    let model = parts
        .model
        .strip_prefix("snowflake/")
        .unwrap_or(parts.model);
    let mut body = Object::from_iter([
        (MODEL.to_owned(), Json::Text(model.to_owned())),
        (
            MESSAGES.to_owned(),
            Json::List(parts.messages.iter().map(project_message).collect()),
        ),
        (STREAM.to_owned(), parts.stream.clone()),
    ]);
    body.extend(params);
    body.extend(parts.extra_body.clone());
    if let Some(value) = resolved {
        body.insert(MAX_COMPLETION_TOKENS.to_owned(), value);
    }
    body
}

// Python's "max_completion_tokens or max_tokens": a falsy first value falls through, and null means no limit
fn resolved_max(max_completion_tokens: Option<Json>, max_tokens: Option<Json>) -> Option<Json> {
    match max_completion_tokens {
        Some(value) if value.is_truthy() => Some(value),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => max_tokens.filter(|value| !matches!(value, Json::Null)),
    }
}

async fn claude_body<Source>(
    parts: &RequestParts<'_>,
    mut params: Object,
    fetcher: &Source,
) -> Result<Object, RequestError>
where
    Source: RemoteMediaFetcher + Sync,
{
    let passthrough_system = params
        .shift_remove(SYSTEM)
        .filter(|value| !matches!(value, Json::Null));
    let split = super::messages::extract_system_and_messages(parts.messages, fetcher).await?;
    let system = passthrough_system.or_else(|| split.system.map(Json::List));
    if params.contains_key(TOOLS) {
        let converted = transform_tools(list_field(&params, TOOLS)?)?;
        params.insert(TOOLS.to_owned(), Json::List(converted));
    }
    if let Some(tool_choice) = params.get(TOOL_CHOICE).cloned() {
        let converted = transform_tool_choice(&tool_choice)?;
        params.insert(TOOL_CHOICE.to_owned(), converted);
    }
    let max_completion_tokens = params.shift_remove(MAX_COMPLETION_TOKENS);
    if let Some(value) = max_completion_tokens.filter(Json::is_truthy)
        && !params.contains_key(MAX_TOKENS)
    {
        params.insert(MAX_TOKENS.to_owned(), value);
    }
    let model = parts
        .model
        .strip_prefix("snowflake/")
        .unwrap_or(parts.model);
    let mut body = Object::from_iter([
        (MODEL.to_owned(), Json::Text(model.to_owned())),
        (MESSAGES.to_owned(), Json::List(split.conversation)),
        (STREAM.to_owned(), parts.stream.clone()),
    ]);
    body.extend(params);
    body.extend(parts.extra_body.clone());
    let mut body = normalize_cache_control(&body);
    if let Some(system) = system {
        let wrapped = Object::from_iter([(SYSTEM.to_owned(), system)]);
        if let Some(normalized) = normalize_cache_control(&wrapped).get(SYSTEM) {
            body.insert(SYSTEM.to_owned(), normalized.clone());
        }
    }
    if !body.contains_key(MAX_TOKENS) {
        body.insert(
            MAX_TOKENS.to_owned(),
            Json::Number(Number::from(DEFAULT_MAX_TOKENS)),
        );
    }
    Ok(body)
}

fn list_field<'params>(
    params: &'params Object,
    field: &'static str,
) -> Result<&'params [Json], RequestError> {
    match params.get(field) {
        Some(Json::List(items)) => Ok(items),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(RequestError::List { field })
        }
    }
}

#[cfg(test)]
mod tests;
