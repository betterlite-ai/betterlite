use betterlite_anthropic::message_replay::cache_control::normalize_cache_control;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock::projection::project_message;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media::inline::async_inline_remote_media;
use betterlite_prompts::media::parts::INLINE_EVERY_REMOTE_URL;
use betterlite_prompts::message_content::Message;
use serde_json::Number;
use thiserror::Error;

use super::Fetcher;
use super::config::is_claude_model;
use super::messages::MessageError;
use super::messages::tools::{ToolError, transform_tool_choice, transform_tools};

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
    Source: Fetcher,
{
    let mut params = optional_params;
    let stream = match params.shift_remove("stream") {
        Some(value) if value.is_truthy() => value,
        Some(_) | None => Json::Bool(false),
    };
    let extra_body = match params.shift_remove("extra_body") {
        None => Object::new(),
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(RequestError::ExtraBody),
    };
    let parts = RequestParts {
        model,
        messages,
        stream,
        extra_body,
    };
    if is_claude_model(model) {
        return claude_body(&parts, params, fetcher).await;
    }
    Ok(openai_body(&parts, params))
}

// LiteLLM inlines remote media only for the Claude dialect; the OpenAI body keeps the URLs as sent
pub async fn async_transform_request<Source>(
    model: &str,
    messages: &[Message],
    optional_params: Object,
    fetcher: &Source,
) -> Result<Object, RequestError>
where
    Source: Fetcher,
{
    if !is_claude_model(model) {
        return transform_request(model, messages, optional_params, fetcher).await;
    }
    let inlined = async_inline_remote_media(messages, &INLINE_EVERY_REMOTE_URL, fetcher).await?;
    transform_request(model, &inlined, optional_params, fetcher).await
}

fn openai_body(parts: &RequestParts<'_>, mut params: Object) -> Object {
    let completion = params.shift_remove(MAX_COMPLETION_TOKENS);
    let plain = params.shift_remove(MAX_TOKENS);
    let resolved = match completion {
        Some(value) if value.is_truthy() => Some(value),
        Some(_) | None => plain.filter(|value| !matches!(value, Json::Null)),
    };
    let model = parts
        .model
        .strip_prefix("snowflake/")
        .unwrap_or(parts.model);
    let mut body = Object::from_iter([
        ("model".to_owned(), Json::Text(model.to_owned())),
        (
            "messages".to_owned(),
            Json::List(parts.messages.iter().map(project_message).collect()),
        ),
        ("stream".to_owned(), parts.stream.clone()),
    ]);
    body.extend(params);
    body.extend(parts.extra_body.clone());
    if let Some(value) = resolved {
        body.insert(MAX_COMPLETION_TOKENS.to_owned(), value);
    }
    body
}

async fn claude_body<Source>(
    parts: &RequestParts<'_>,
    mut params: Object,
    fetcher: &Source,
) -> Result<Object, RequestError>
where
    Source: Fetcher,
{
    let passthrough = params
        .shift_remove("system")
        .filter(|value| !matches!(value, Json::Null));
    let split = super::messages::extract_system_and_messages(parts.messages, fetcher).await?;
    let system = passthrough.or_else(|| split.system.map(Json::List));
    if params.contains_key("tools") {
        let Some(Json::List(tools)) = params.get("tools") else {
            return Err(RequestError::List { field: "tools" });
        };
        let transformed = transform_tools(tools)?;
        params.insert("tools".to_owned(), Json::List(transformed));
    }
    if let Some(tool_choice) = params.get("tool_choice").cloned() {
        params.insert(
            "tool_choice".to_owned(),
            transform_tool_choice(&tool_choice)?,
        );
    }
    if let Some(value) = params
        .shift_remove(MAX_COMPLETION_TOKENS)
        .filter(Json::is_truthy)
        && !params.contains_key(MAX_TOKENS)
    {
        params.insert(MAX_TOKENS.to_owned(), value);
    }
    let model = parts
        .model
        .strip_prefix("snowflake/")
        .unwrap_or(parts.model);
    let mut body = Object::from_iter([
        ("model".to_owned(), Json::Text(model.to_owned())),
        ("messages".to_owned(), Json::List(split.conversation)),
        ("stream".to_owned(), parts.stream.clone()),
    ]);
    body.extend(params);
    body.extend(parts.extra_body.clone());
    let normalized = normalize_cache_control(&body);
    let mut finished = with_system(normalized, system);
    if !finished.contains_key(MAX_TOKENS) {
        finished.insert(
            MAX_TOKENS.to_owned(),
            Json::Number(Number::from(DEFAULT_MAX_TOKENS)),
        );
    }
    Ok(finished)
}

// system is added to the body after the cache control normalization, so it is normalized on its own
fn with_system(mut body: Object, system: Option<Json>) -> Object {
    let Some(present) = system else {
        return body;
    };
    let wrapped = Object::from_iter([("system".to_owned(), present)]);
    if let Some(normalized) = normalize_cache_control(&wrapped).get("system") {
        body.insert("system".to_owned(), normalized.clone());
    }
    body
}

#[cfg(test)]
mod tests;
