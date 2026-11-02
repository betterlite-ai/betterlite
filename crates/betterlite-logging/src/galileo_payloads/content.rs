use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use super::messages::input_text_from_messages;
use super::support::has_value;
use crate::python_json;

mod families;
mod model_response;
mod responses_output;

// Shared by the chat image-list and the Responses-API output-list projections: a List runs each item through `project`, anything else passes through unmodified.
fn project_list_or_passthrough<F>(value: &Json, project: F) -> Json
where
    F: Fn(&Json) -> Json,
{
    match value {
        Json::List(items) => Json::List(items.iter().map(project).collect()),
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            other.clone()
        }
    }
}

// Mirrors the level string _get_galileo_input_output_content receives from the callback dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Default,
    Error,
}

// Python resolves the response family with isinstance; the oracle helper names it explicitly through the `family` tag instead.
#[derive(Deserialize)]
#[serde(tag = "family", rename_all = "snake_case")]
pub enum ResponseObj {
    Chat {
        #[serde(default)]
        message: Option<Json>,
    },
    Embedding,
    Speech,
    TextCompletion {
        #[serde(default)]
        text: Option<String>,
    },
    Image {
        #[serde(default)]
        data: Option<Json>,
    },
    Transcription {
        #[serde(default)]
        text: Option<String>,
    },
    Rerank {
        results: Json,
    },
    ResponsesApi {
        output: Json,
    },
    RealtimeList {
        #[serde(default)]
        items: Vec<Json>,
    },
    PassThroughDict {
        #[serde(default)]
        value: IndexMap<String, Json>,
    },
    GenericDict {
        #[serde(default)]
        value: IndexMap<String, Json>,
    },
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct GalileoKwargs {
    #[serde(default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub messages: Option<Json>,
    #[serde(default)]
    pub functions: Option<Json>,
    #[serde(default)]
    pub tools: Option<Json>,
    #[serde(default)]
    pub input: Option<Json>,
}

// litellm.integrations.galileo.GalileoObserve._build_prompt.
#[must_use]
pub fn build_prompt(kwargs: &GalileoKwargs) -> IndexMap<String, Json> {
    let mut prompt = IndexMap::new();
    prompt.insert(
        "messages".to_owned(),
        kwargs.messages.clone().unwrap_or(Json::Null),
    );
    if let Some(functions) = &kwargs.functions {
        prompt.insert("functions".to_owned(), functions.clone());
    }
    if let Some(tools) = &kwargs.tools {
        prompt.insert("tools".to_owned(), tools.clone());
    }
    prompt
}

// litellm.integrations.galileo.GalileoObserve._serialize_galileo_output; every value this layer carries is already plain JSON, so the hasattr(obj, "model_dump") branch never applies here.
#[must_use]
pub fn serialize_galileo_output(value: Option<&Json>) -> String {
    match value {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(text)) => text.clone(),
        Some(other @ (Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))) => {
            python_json::dumps(other)
        }
    }
}

// litellm.integrations.galileo.GalileoObserve._prompt_to_input_text.
#[must_use]
pub fn prompt_to_input_text(prompt: &IndexMap<String, Json>) -> String {
    if let Some(messages) = prompt.get("messages")
        && !matches!(messages, Json::Null)
    {
        let text = input_text_from_messages(messages);
        if !text.is_empty() {
            return text;
        }
    }
    python_json::dumps_object(prompt)
}

// litellm.integrations.galileo.GalileoObserve._langfuse_style_rerank_prompt.
#[must_use]
pub fn langfuse_style_rerank_prompt(kwargs: &GalileoKwargs) -> IndexMap<String, Json> {
    let mut prompt = IndexMap::new();
    prompt.insert(
        "messages".to_owned(),
        kwargs.messages.clone().unwrap_or(Json::Null),
    );
    prompt
}

fn messages_or_empty(kwargs: &GalileoKwargs) -> Json {
    match &kwargs.messages {
        Some(value) if has_value(value) => value.clone(),
        _ => Json::List(Vec::new()),
    }
}

// litellm.integrations.galileo.GalileoObserve._get_galileo_input_output_content; returns (input_text, output_text, messages_for_span).
#[must_use]
pub fn galileo_input_output_content(
    kwargs: &GalileoKwargs,
    response: Option<&ResponseObj>,
    level: Level,
    status_message: Option<&str>,
) -> (String, String, Json) {
    let prompt = build_prompt(kwargs);

    if level == Level::Error
        && let Some(message) = status_message
    {
        return (
            prompt_to_input_text(&prompt),
            message.to_owned(),
            Json::Object(prompt),
        );
    }

    if response.is_some()
        && (matches!(
            kwargs.call_type.as_deref(),
            Some("embedding" | "aembedding")
        ) || matches!(response, Some(ResponseObj::Embedding)))
    {
        return (
            prompt_to_input_text(&prompt),
            "embedding-output".to_owned(),
            Json::Object(prompt),
        );
    }

    let Some(resolved_response) = response else {
        return (
            prompt_to_input_text(&prompt),
            String::new(),
            messages_or_empty(kwargs),
        );
    };

    families::response_content(&prompt, kwargs, resolved_response)
}
