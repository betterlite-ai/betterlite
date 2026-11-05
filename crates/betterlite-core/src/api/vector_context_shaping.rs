#[path = "vector_context_shaping/tests.rs"]
mod tests;

use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;

const GUARDRAIL_KEYS: [&str; 4] = [
    "guardrails",
    "guardrail_config",
    "policies",
    "include_guardrail_response",
];
const CONTEXT_PREFIX: &str = "Context:\n\n";
const SUPPORTED_FAILURE_MODES: &str = "annotate, error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureMode {
    Annotate,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureModeResolution {
    pub mode: FailureMode,
    pub warning: Option<String>,
}

#[must_use]
pub fn scan_request(
    model: &str,
    non_default_params: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let Some(client_body) = proxy_client_body(non_default_params) else {
        let mut fallback = non_default_params.clone();
        fallback.insert("model".to_owned(), Json::Text(model.to_owned()));
        return fallback;
    };
    let mut merged = client_body.clone();
    merged.extend(non_default_params.clone());
    merged.retain(|key, _| !GUARDRAIL_KEYS.contains(&key.as_str()));
    merged
}

fn proxy_client_body(
    non_default_params: &IndexMap<String, Json>,
) -> Option<&IndexMap<String, Json>> {
    let Json::Object(proxy_request) = non_default_params.get("proxy_server_request")? else {
        return None;
    };
    let Json::Object(body) = proxy_request.get("body")? else {
        return None;
    };
    Some(body)
}

#[must_use]
pub fn extract_query_from_messages(messages: &[ChatMessage]) -> Option<String> {
    match messages.last()?.content.as_ref()? {
        MessageContent::Text(text) => Some(text.clone()),
        MessageContent::Parts(parts) => parts.iter().find_map(first_text_part),
    }
}

fn first_text_part(part: &ContentPart) -> Option<String> {
    if part.kind != "text" {
        return None;
    }
    part.fields.get("text")?.as_str().map(str::to_owned)
}

#[must_use]
pub fn context_message(search_response: &VectorStoreSearchResponse) -> Option<ChatMessage> {
    let data = search_response.data.as_ref()?;
    let texts: Vec<&str> = data
        .iter()
        .filter_map(|result| result.content.as_deref())
        .flatten()
        .filter_map(|content_item| {
            let text = content_item.text.as_deref()?;
            (!text.is_empty()).then_some(text)
        })
        .collect();
    if texts.is_empty() {
        return None;
    }
    let mut body = String::new();
    for text in &texts {
        body.push_str(text);
        body.push_str("\n\n");
    }
    Some(ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Text(format!("{CONTEXT_PREFIX}{body}"))),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    })
}

#[must_use]
pub fn context_messages(search_results: &[VectorStoreSearchResponse]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for search_response in search_results {
        messages.extend(context_message(search_response));
    }
    messages
}

#[must_use]
pub fn messages_with_context(
    messages: &[ChatMessage],
    context_messages: &[ChatMessage],
) -> Vec<ChatMessage> {
    if context_messages.is_empty() {
        return messages.to_vec();
    }
    let split = messages.len().saturating_sub(1);
    let (before_last, last) = messages.split_at(split);
    let mut result = before_last.to_vec();
    result.extend(context_messages.iter().cloned());
    result.extend(last.iter().cloned());
    result
}

#[must_use]
pub fn requested_vector_store_ids(non_default_params: &IndexMap<String, Json>) -> Vec<String> {
    let Some(Json::List(requested)) = non_default_params.get("vector_store_ids") else {
        return Vec::new();
    };
    requested.iter().map(value_text).collect()
}

#[must_use]
pub fn configured_failure_mode(configured: &Json) -> FailureModeResolution {
    match configured {
        Json::Text(text) if text == "annotate" => FailureModeResolution {
            mode: FailureMode::Annotate,
            warning: None,
        },
        Json::Text(text) if text == "error" => FailureModeResolution {
            mode: FailureMode::Error,
            warning: None,
        },
        Json::Null
        | Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_) => FailureModeResolution {
            mode: FailureMode::Annotate,
            warning: Some(format!(
                "Unsupported vector_store_search_failure_mode={}, falling back to 'annotate'. Supported modes: {SUPPORTED_FAILURE_MODES}",
                value_repr(configured)
            )),
        },
    }
}

fn value_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::List(_) | Json::Object(_) => format!("{value:?}"),
    }
}

fn value_repr(value: &Json) -> String {
    if let Json::Text(text) = value {
        format!("'{text}'")
    } else {
        value_text(value)
    }
}
