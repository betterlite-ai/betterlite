use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Message, convert_message_content_lists};
use indexmap::IndexMap;
use serde_json::{Number, Value};
use thiserror::Error;

use crate::chat::{ChatMessage, MessageContent, Role};
use crate::chat_content::{ChatContent, ChatContentError, Mode, UrlDownloader};
use crate::edenai_chat::ReasoningLookup;
use crate::openai_base_config::CatalogCapabilities;

pub const PROVIDER: &str = "moonshot";

const DEFAULT_API_BASE: &str = "https://api.moonshot.ai/v1";
const CHAT_COMPLETIONS: &str = "/chat/completions";
const KIMI_THINKING_PREVIEW: &str = "kimi-thinking-preview";
const KIMI_EXCLUDED: [&str; 3] = ["functions", "tools", "tool_choice"];
const FUNCTIONS_EXCLUDED: [&str; 1] = ["functions"];
const MAX_COMPLETION_TOKENS: &str = "max_completion_tokens";
const MAX_TOKENS: &str = "max_tokens";
const REASONING_EFFORT: &str = "reasoning_effort";
const EFFORT: &str = "effort";
const TEMPERATURE: &str = "temperature";
const COUNT: &str = "n";
const TOOL_CHOICE: &str = "tool_choice";
const REQUIRED: &str = "required";
const TEXT: &str = "text";
const REASONING_CONTENT: &str = "reasoning_content";
const PROVIDER_SPECIFIC_FIELDS: &str = "provider_specific_fields";
const TOOL_SELECTION_TEXT: &str = "Please select a tool to handle the current issue.";

#[derive(Debug, Error)]
pub enum MoonshotError {
    #[error(transparent)]
    Content(#[from] ChatContentError),
}

impl MoonshotError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Content(error) => error.status_code(),
        }
    }
}

pub struct TransformInput<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request mut Object,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

// no base, or an empty one, gives the default; a base already ending in the path is kept as is
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    if base.ends_with(CHAT_COMPLETIONS) {
        base.to_owned()
    } else {
        format!("{base}{CHAT_COMPLETIONS}")
    }
}

// functions is never supported, the thinking model loses tool calls too, and a reasoning model adds reasoning_effort
#[must_use]
pub fn supported_params<Lookup>(
    lookup: &Lookup,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Vec<&'static str>
where
    Lookup: ReasoningLookup,
{
    let excluded: &[&str] = if model.contains(KIMI_THINKING_PREVIEW) {
        &KIMI_EXCLUDED
    } else {
        &FUNCTIONS_EXCLUDED
    };
    let mut params: Vec<&'static str> = super::openai_parent()
        .supported_params(model, catalog)
        .into_iter()
        .filter(|param| !excluded.contains(param))
        .collect();
    if lookup.supports_reasoning(model).unwrap_or(false) {
        params.push(REASONING_EFFORT);
    }
    params
}

// the walk keeps the caller's order: max_completion_tokens becomes max_tokens, and an unsupported param is skipped
pub fn map_openai_params<Lookup>(
    lookup: &Lookup,
    model: &str,
    non_default_params: &Object,
    optional_params: &mut Object,
    catalog: &CatalogCapabilities,
) where
    Lookup: ReasoningLookup,
{
    let supported = supported_params(lookup, model, catalog);
    for (name, value) in non_default_params {
        let renamed = name == MAX_COMPLETION_TOKENS;
        let supported_here = supported.contains(&name.as_str());
        let target = if renamed { MAX_TOKENS } else { name.as_str() };
        let mapped = if renamed {
            Some(value.clone())
        } else if supported_here {
            if name == REASONING_EFFORT {
                reasoning_effort_value(value)
            } else {
                Some(value.clone())
            }
        } else {
            None
        };
        if let Some(mapped_value) = mapped {
            optional_params.insert(target.to_owned(), mapped_value);
        }
    }
    if lookup.supports_reasoning(model).unwrap_or(false) {
        optional_params.shift_remove(TEMPERATURE);
        return;
    }
    cap_temperature(optional_params);
}

// only the effort string survives, from a bare string or an object's effort field
fn reasoning_effort_value(value: &Json) -> Option<Json> {
    let effort = match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Object(fields) => fields.get(EFFORT).and_then(Json::as_str),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    };
    effort.map(|effort_text| Json::Text(effort_text.to_owned()))
}

// KIMI takes [0, 1]; below 0.3 with n above 1 the API raises, so 0.3 is sent instead
fn cap_temperature(optional_params: &mut Object) {
    let Some(temperature) = optional_params.get(TEMPERATURE) else {
        return;
    };
    let Some(current) = super::as_f64(temperature) else {
        return;
    };
    let capped = if current > 1.0_f64 {
        optional_params.insert(TEMPERATURE.to_owned(), Json::Number(Number::from(1_u64)));
        1.0_f64
    } else {
        current
    };
    if capped < 0.3_f64 && n_above_one(optional_params) {
        let floor = serde_json::from_str::<Json>("0.3").unwrap_or(Json::Null);
        optional_params.insert(TEMPERATURE.to_owned(), floor);
    }
}

fn n_above_one(optional_params: &Object) -> bool {
    optional_params
        .get(COUNT)
        .is_some_and(|count| super::as_f64(count).is_some_and(|value| value > 1.0_f64))
}

// tool_choice "required" becomes a user message, a reasoning model gets reasoning_content filled, and the parent runs
pub fn transform_request<Lookup>(
    lookup: &Lookup,
    input: TransformInput<'_>,
    mode: Mode,
) -> Result<Object, MoonshotError>
where
    Lookup: ReasoningLookup,
{
    let TransformInput {
        model,
        messages,
        optional_params,
        environment,
        downloads,
    } = input;
    let mut prepared = messages.to_vec();
    if optional_params.get(TOOL_CHOICE).and_then(Json::as_str) == Some(REQUIRED) {
        optional_params.shift_remove(TOOL_CHOICE);
        prepared.push(ChatMessage {
            role: Role::User,
            content: Some(MessageContent::Text(String::from(TOOL_SELECTION_TEXT))),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            passthrough: IndexMap::new(),
        });
    }
    if lookup.supports_reasoning(model).unwrap_or(false) {
        prepared = fill_reasoning_content(&prepared);
    }
    let flattened = flatten_text_lists(&prepared)?;
    let content = ChatContent {
        model,
        messages: &flattened,
        optional_params,
        provider: Some(PROVIDER),
        api_base: None,
        global_api_base: None,
        openai_system_messages_first: false,
        environment,
        downloads,
    };
    Ok(content.transform(mode)?.to_body()?)
}

// a non-text part anywhere leaves every list content alone, even in another message
fn flatten_text_lists(messages: &[ChatMessage]) -> Result<Vec<ChatMessage>, MoonshotError> {
    if messages.iter().any(has_non_text_part) {
        return Ok(messages.to_vec());
    }
    let value = serde_json::to_value(messages)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    let mut prompt: Vec<Message> = serde_json::from_value(value)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    let converted = convert_message_content_lists(&mut prompt);
    let converted_value = serde_json::to_value(converted)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    let flattened = serde_json::from_value(converted_value)
        .map_err(|source| ChatContentError::MessageShape { source })?;
    Ok(flattened)
}

fn has_non_text_part(message: &ChatMessage) -> bool {
    match message.content.as_ref() {
        Some(MessageContent::Parts(parts)) => parts.iter().any(|part| part.kind != TEXT),
        Some(MessageContent::Text(_)) | None => false,
    }
}

// an assistant message with tool calls needs reasoning_content: the stored value moves up, else a space does
fn fill_reasoning_content(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut filled = Vec::with_capacity(messages.len());
    for message in messages {
        let has_tool_calls = message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty());
        let has_reasoning = message
            .passthrough
            .get(REASONING_CONTENT)
            .is_some_and(super::json_is_truthy);
        if message.role != Role::Assistant || !has_tool_calls || has_reasoning {
            filled.push(message.clone());
            continue;
        }
        let mut patched = message.clone();
        if let Some(stored) = stored_reasoning_content(&message.passthrough) {
            patched
                .passthrough
                .insert(REASONING_CONTENT.to_owned(), stored);
            if let Some(Value::Object(fields)) =
                patched.passthrough.get_mut(PROVIDER_SPECIFIC_FIELDS)
            {
                fields.remove(REASONING_CONTENT);
            }
        } else {
            tracing::warn!(
                "Moonshot reasoning model: assistant tool-call message is missing `reasoning_content`. Injecting a placeholder to satisfy API validation. For best results, preserve `reasoning_content` from the original assistant response when building multi-turn conversation history."
            );
            patched.passthrough.insert(
                REASONING_CONTENT.to_owned(),
                Value::String(String::from(" ")),
            );
        }
        filled.push(patched);
    }
    filled
}

fn stored_reasoning_content(passthrough: &IndexMap<String, Value>) -> Option<Value> {
    let fields = passthrough.get(PROVIDER_SPECIFIC_FIELDS)?.as_object()?;
    let stored = fields.get(REASONING_CONTENT)?;
    super::json_is_truthy(stored).then(|| stored.clone())
}
