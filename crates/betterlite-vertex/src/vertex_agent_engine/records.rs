use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::{Extra, Json};
use betterlite_models::usage_sum::Usage;
use betterlite_prompts::message_content::{Message, convert_content_list_to_str};
use betterlite_tokenizer::count::error::CountError;
use serde::Serialize;
use uuid::Uuid;

use super::error::AgentEngineError;
use crate::model_urls::base_url::get_vertex_base_url;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["user"];
pub const DEFAULT_LOCATION: &str = "us-central1";
// Agent Engine answers with its own SSE stream even for a buffered request, so a custom wrapper reads it
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
// the :streamQuery endpoint alone selects streaming, so the body never carries a stream field
pub const STREAM_PARAM_IN_BODY: bool = false;
// the answer already arrives as a stream, so streaming is never faked from one buffered body
pub const SHOULD_FAKE_STREAM: bool = false;
// B196 counts with the gpt-3.5-turbo encoding whatever agent model the request names
const TOKEN_MODEL: &str = "gpt-3.5-turbo";
const USER_ID_FIELD: &str = "user_id";
const USER_FIELD: &str = "user";

#[must_use]
pub fn map_openai_params(non_default_params: &Extra, mut optional_params: Extra) -> Extra {
    if let Some(user) = non_default_params.get(USER_FIELD) {
        optional_params.insert(USER_ID_FIELD.to_owned(), user.clone());
    }
    optional_params
}

#[derive(Debug, Clone, Copy)]
pub struct AgentEngineTarget<'call> {
    pub model: &'call str,
    pub project: Option<&'call str>,
    pub location: Option<&'call str>,
}

#[must_use]
pub fn complete_url(target: &AgentEngineTarget<'_>) -> Result<String, AgentEngineError> {
    let location = target
        .location
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LOCATION);
    // a full projects/ resource path names its own location, and only the host comes from configuration
    let parsed = target
        .model
        .strip_prefix("agent_engine/")
        .unwrap_or(target.model);
    let resource = if parsed.starts_with("projects/") {
        parsed.to_owned()
    } else {
        let project = target
            .project
            .filter(|value| !value.is_empty())
            .ok_or(AgentEngineError::ProjectRequired)?;
        format!("projects/{project}/locations/{location}/reasoningEngines/{parsed}")
    };
    let base = get_vertex_base_url(Some(location)).map_err(AgentEngineError::Location)?;
    Ok(format!("{base}/v1beta1/{resource}:streamQuery"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentQueryInput {
    pub message: String,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentQueryPayload {
    pub class_method: &'static str,
    pub input: AgentQueryInput,
}

pub fn transform_request(
    messages: &[Message],
    optional_params: &Extra,
    generated_user: Uuid,
) -> Result<AgentQueryPayload, AgentEngineError> {
    let last = messages.last().ok_or(AgentEngineError::EmptyMessages)?;
    let message = convert_content_list_to_str(last);
    let mut user_id = text_param(optional_params, USER_ID_FIELD)?;
    if user_id.is_none() {
        user_id = text_param(optional_params, USER_FIELD)?;
    }
    Ok(AgentQueryPayload {
        class_method: "stream_query",
        input: AgentQueryInput {
            message,
            user_id: user_id.unwrap_or_else(|| generated_user_id(generated_user)),
            session_id: text_param(optional_params, "session_id")?,
        },
    })
}

// LiteLLM reads user_id then user, takes only a truthy one, and generates an id when neither is set
fn text_param(
    optional_params: &Extra,
    field: &'static str,
) -> Result<Option<String>, AgentEngineError> {
    match optional_params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(AgentEngineError::FieldType { field })
        }
    }
}

fn generated_user_id(generated: Uuid) -> String {
    let head: String = generated.to_string().chars().take(8).collect();
    format!("litellm-user-{head}")
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAnswer {
    pub model: String,
    pub index: u32,
    pub finish_reason: &'static str,
    pub role: &'static str,
    pub content: String,
    pub usage: Option<Usage>,
}
// The buffered answer is an SSE body LiteLLM folds into its last non-empty text snapshot
pub fn decode_vertex_agent_buffer(
    status: u16,
    body: &[u8],
    model: &str,
) -> Result<AgentAnswer, AgentEngineError> {
    let text =
        std::str::from_utf8(body).map_err(|source| AgentEngineError::NotUtf8(status, source))?;
    let mut content = String::new();
    for line in text.trim().lines().map(str::trim) {
        if let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(line) {
            let found = extract_text(&fields);
            if !found.is_empty() {
                content = found;
            }
        }
    }
    Ok(AgentAnswer {
        model: model.to_owned(),
        index: 0,
        finish_reason: "stop",
        role: "assistant",
        content,
        usage: None,
    })
}

// the first part carrying a text key decides, and only an object without one falls back to state_delta
fn extract_text(fields: &Extra) -> String {
    if let Some(Json::Object(content)) = fields.get("content")
        && let Some(Json::List(parts)) = content.get("parts")
    {
        for part in parts {
            if let Json::Object(part) = part
                && let Some(text) = part.get("text")
            {
                return match text {
                    Json::Text(text) => text.clone(),
                    _ => String::new(),
                };
            }
        }
    }
    if let Some(Json::Object(actions)) = fields.get("actions")
        && let Some(Json::Object(state_delta)) = actions.get("state_delta")
        && let Some(Json::Text(text)) = state_delta
            .values()
            .find(|value| matches!(value, Json::Text(text) if !text.is_empty()))
    {
        return text.clone();
    }
    String::new()
}
pub trait TokenCounter: Send + Sync {
    fn count_messages<'count>(
        &'count self,
        model: &'count str,
        messages: &'count [Message],
    ) -> impl Future<Output = Result<u64, CountError>> + Send + 'count;

    fn count_response_tokens<'count>(
        &'count self,
        model: &'count str,
        text: &'count str,
    ) -> impl Future<Output = Result<u64, CountError>> + Send + 'count;
}

pub async fn transform_response<Counter>(
    status: u16,
    body: &[u8],
    model: &str,
    messages: &[Message],
    counter: &Counter,
) -> Result<AgentAnswer, AgentEngineError>
where
    Counter: TokenCounter + Sync,
{
    let mut answer = decode_vertex_agent_buffer(status, body, model)?;
    match count_usage(&answer.content, messages, counter).await {
        Ok(usage) => answer.usage = Some(usage),
        Err(error) => tracing::warn!("Failed to calculate token usage: {error}"),
    }
    Ok(answer)
}

async fn count_usage<Counter>(
    content: &str,
    messages: &[Message],
    counter: &Counter,
) -> Result<Usage, CountError>
where
    Counter: TokenCounter + Sync,
{
    let prompt = counter.count_messages(TOKEN_MODEL, messages).await?;
    let completion = counter.count_response_tokens(TOKEN_MODEL, content).await?;
    let prompt_tokens = i64::try_from(prompt).map_err(|_| CountError::Overflow)?;
    let completion_tokens = i64::try_from(completion).map_err(|_| CountError::Overflow)?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(CountError::Overflow)?;
    Ok(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    })
}
mod tests;
