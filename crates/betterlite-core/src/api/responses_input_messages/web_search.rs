use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

use crate::api::responses_input_messages::content_message;

const PREFIX: &str = "Hosted web search: ";
const CALL_FIELDS: [&str; 4] = ["id", "action", "status", "type"];
const SEARCH_FIELDS: [&str; 4] = ["type", "queries", "query", "sources"];
const OPEN_PAGE_FIELDS: [&str; 2] = ["type", "url"];
const FIND_FIELDS: [&str; 3] = ["type", "pattern", "url"];
const SOURCE_FIELDS: [&str; 2] = ["type", "url"];
const STATUSES: [&str; 4] = ["in_progress", "searching", "completed", "failed"];

#[derive(Debug, Error)]
pub enum WebSearchError {
    #[error("the web search call needs {field} as a string")]
    Missing { field: &'static str },
    #[error("the web search call field {field} must be a string")]
    NotText { field: &'static str },
    #[error("the web search call field {field} must be a list")]
    NotList { field: &'static str },
    #[error("the web search call field {field} must be an object")]
    NotObject { field: &'static str },
    #[error("the web search call status must be in_progress, searching, completed or failed")]
    Status,
    #[error("the web search action type must be search, open_page or find_in_page")]
    ActionType,
    #[error("the web search source type must be url")]
    SourceType,
    #[error("the web search call cannot be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Serialize)]
struct HostedCall<'call> {
    id: &'call str,
    action: Action<'call>,
    status: &'call str,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Action<'call> {
    Search(SearchAction<'call>),
    OpenPage(OpenPageAction<'call>),
    Find(FindAction<'call>),
}

#[derive(Serialize)]
struct SearchAction<'call> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    queries: Option<Vec<&'call str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query: Option<&'call str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sources: Option<Vec<SourceAction<'call>>>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Serialize)]
struct OpenPageAction<'call> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'call str>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Serialize)]
struct FindAction<'call> {
    pattern: &'call str,
    #[serde(rename = "type")]
    kind: &'static str,
    url: &'call str,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Serialize)]
struct SourceAction<'call> {
    #[serde(rename = "type")]
    kind: &'static str,
    url: &'call str,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

pub(crate) fn hosted_search_message(
    fields: &IndexMap<String, Json>,
) -> Result<Value, WebSearchError> {
    let call = HostedCall {
        id: text(fields, "id")?,
        action: action(fields.get("action"))?,
        status: status(fields.get("status"))?,
        kind: "web_search_call",
        extra: extras(fields, &CALL_FIELDS)?,
    };
    let dump =
        serde_json::to_string(&call).map_err(|source| WebSearchError::Serialize { source })?;
    Ok(content_message(
        "assistant",
        Value::String(format!("{PREFIX}{dump}")),
    ))
}

fn action(value: Option<&Json>) -> Result<Action<'_>, WebSearchError> {
    let Some(Json::Object(fields)) = value else {
        return Err(WebSearchError::NotObject { field: "action" });
    };
    let kind = match fields.get("type") {
        Some(Json::Text(kind)) => kind.as_str(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(WebSearchError::ActionType);
        }
    };
    match kind {
        "search" => Ok(Action::Search(SearchAction {
            kind: "search",
            queries: text_list(fields, "queries")?,
            query: optional_text(fields, "query")?,
            sources: sources(fields)?,
            extra: extras(fields, &SEARCH_FIELDS)?,
        })),
        "open_page" => Ok(Action::OpenPage(OpenPageAction {
            kind: "open_page",
            url: optional_text(fields, "url")?,
            extra: extras(fields, &OPEN_PAGE_FIELDS)?,
        })),
        "find_in_page" => Ok(Action::Find(FindAction {
            pattern: text(fields, "pattern")?,
            kind: "find_in_page",
            url: text(fields, "url")?,
            extra: extras(fields, &FIND_FIELDS)?,
        })),
        _ => Err(WebSearchError::ActionType),
    }
}

fn sources(
    fields: &IndexMap<String, Json>,
) -> Result<Option<Vec<SourceAction<'_>>>, WebSearchError> {
    match fields.get("sources") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => items
            .iter()
            .map(source)
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(WebSearchError::NotList { field: "sources" })
        }
    }
}

fn source(value: &Json) -> Result<SourceAction<'_>, WebSearchError> {
    let Json::Object(fields) = value else {
        return Err(WebSearchError::NotObject { field: "sources" });
    };
    if !matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "url") {
        return Err(WebSearchError::SourceType);
    }
    Ok(SourceAction {
        kind: "url",
        url: text(fields, "url")?,
        extra: extras(fields, &SOURCE_FIELDS)?,
    })
}

fn status(value: Option<&Json>) -> Result<&str, WebSearchError> {
    let Some(Json::Text(status)) = value else {
        return Err(WebSearchError::Status);
    };
    if !STATUSES.contains(&status.as_str()) {
        return Err(WebSearchError::Status);
    }
    Ok(status)
}

fn text<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<&'fields str, WebSearchError> {
    match fields.get(name) {
        Some(Json::Text(text)) => Ok(text),
        None => Err(WebSearchError::Missing { field: name }),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(WebSearchError::NotText { field: name })
        }
    }
}

fn optional_text<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'fields str>, WebSearchError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(WebSearchError::NotText { field: name })
        }
    }
}

fn text_list<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<Vec<&'fields str>>, WebSearchError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Text(text) => Ok(text.as_str()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    Err(WebSearchError::NotList { field: name })
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Err(WebSearchError::NotList { field: name })
        }
    }
}

fn extras(
    fields: &IndexMap<String, Json>,
    declared: &[&str],
) -> Result<IndexMap<String, Value>, WebSearchError> {
    fields
        .iter()
        .filter(|(name, value)| !declared.contains(&name.as_str()) && !matches!(value, Json::Null))
        .map(|(name, value)| {
            serde_json::to_value(value)
                .map(|wire| (name.clone(), wire))
                .map_err(|source| WebSearchError::Serialize { source })
        })
        .collect()
}
