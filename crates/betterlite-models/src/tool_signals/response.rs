use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::anthropic_pricing::probe::search_requests_from_response;
use crate::cost_usage::CostUsage;
use crate::cost_usage::server_tool_projection::ServerToolUse;
use crate::cost_usage::tool_projection::ToolUsage;
use crate::decisions::Json;

// Only a pydantic chat response, a pydantic Responses response, a raw mapping or another object is a distinct kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBillingResponse {
    Chat(ChatView),
    Responses(ResponsesView),
    Raw(RawView),
    Other(OtherView),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatView {
    #[serde(default)]
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<ToolUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatChoice {
    #[serde(default)]
    pub message: Option<ChatMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    #[serde(default)]
    pub annotations: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    #[serde(default, rename = "type")]
    pub annotation_type: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponsesView {
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(default)]
    pub tool_usage: Option<Json>,
    #[serde(default)]
    pub usage: Option<ToolUsage>,
}

// A Responses output item is either a plain mapping or a parsed object; only a string "type" names either kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputItem {
    Mapping(IndexMap<String, Json>),
    Attribute(IndexMap<String, Json>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawView {
    pub body: Json,
    #[serde(default)]
    pub usage: Option<ToolUsage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherView {
    #[serde(default)]
    pub usage: Option<ToolUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationType {
    UrlCitation,
    FileCitation,
}

impl AnnotationType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::UrlCitation => "url_citation",
            Self::FileCitation => "file_citation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputItemType {
    WebSearchCall,
    FileSearchCall,
}

impl OutputItemType {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::WebSearchCall => "web_search_call",
            Self::FileSearchCall => "file_search_call",
        }
    }
}

impl ToolBillingResponse {
    #[must_use]
    pub const fn usage(&self) -> Option<&ToolUsage> {
        match self {
            Self::Chat(view) => view.usage.as_ref(),
            Self::Responses(view) => view.usage.as_ref(),
            Self::Raw(view) => view.usage.as_ref(),
            Self::Other(view) => view.usage.as_ref(),
        }
    }
}

impl OutputItem {
    pub(crate) fn item_type(&self) -> Option<&str> {
        let fields = match self {
            Self::Mapping(fields) | Self::Attribute(fields) => fields,
        };
        match fields.get("type") {
            Some(Json::Text(text)) => Some(text),
            None
            | Some(
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => None,
        }
    }
}

#[must_use]
pub fn includes_annotation_type(
    response: &ToolBillingResponse,
    annotation_type: AnnotationType,
) -> bool {
    let ToolBillingResponse::Chat(view) = response else {
        return false;
    };
    for choice in &view.choices {
        let Some(message) = &choice.message else {
            continue;
        };
        for annotation in message.annotations.iter().flatten() {
            if matches!(
                annotation.annotation_type.as_ref(),
                Some(Json::Text(text)) if text == annotation_type.as_str()
            ) {
                return true;
            }
        }
    }
    false
}

#[must_use]
pub fn includes_output_type(response: &ToolBillingResponse, output_type: OutputItemType) -> bool {
    let ToolBillingResponse::Responses(view) = response else {
        return false;
    };
    view.output
        .iter()
        .any(|item| item.item_type() == Some(output_type.as_str()))
}

#[must_use]
pub fn includes_web_search_call(response: &ToolBillingResponse) -> bool {
    match response {
        ToolBillingResponse::Chat(view) => {
            includes_annotation_type(response, AnnotationType::UrlCitation)
                || usage_signals(view.usage.as_ref())
        }
        ToolBillingResponse::Responses(_) => {
            includes_output_type(response, OutputItemType::WebSearchCall)
        }
        ToolBillingResponse::Raw(view) => {
            search_requests_from_response(&view.body).is_some()
                || usage_signals(view.usage.as_ref())
        }
        ToolBillingResponse::Other(view) => usage_signals(view.usage.as_ref()),
    }
}

#[must_use]
pub fn includes_file_search_call(response: &ToolBillingResponse) -> bool {
    if matches!(response, ToolBillingResponse::Chat(_)) {
        return includes_annotation_type(response, AnnotationType::FileCitation);
    }
    includes_output_type(response, OutputItemType::FileSearchCall)
}

// The raw probe always wins; a typed chat or Responses object never falls through to the supplied usage.
#[must_use]
pub fn anthropic_search_usage(response: &ToolBillingResponse) -> Option<ToolUsage> {
    let supplied = response.usage();
    if supplied
        .and_then(|existing| existing.server_tool_use)
        .and_then(|server_tool_use| server_tool_use.web_search_requests)
        .is_some()
    {
        return supplied.cloned();
    }
    let reported = match response {
        ToolBillingResponse::Raw(view) => search_requests_from_response(&view.body),
        ToolBillingResponse::Chat(_)
        | ToolBillingResponse::Responses(_)
        | ToolBillingResponse::Other(_) => None,
    };
    let Some(count) = reported else {
        return supplied.cloned();
    };
    let server_tool_use = ServerToolUse {
        web_search_requests: Some(count),
        tool_search_requests: None,
        browser_open_requests: None,
    };
    Some(supplied.map_or_else(
        || ToolUsage {
            cost: CostUsage::default(),
            server_tool_use: Some(server_tool_use),
            server_side_details: None,
        },
        |existing| existing.with_server_tool_use(server_tool_use),
    ))
}

fn usage_signals(usage: Option<&ToolUsage>) -> bool {
    let Some(supplied) = usage else {
        return false;
    };
    let prompt_signal = supplied
        .cost
        .prompt_details
        .and_then(|details| details.web_search_requests)
        .is_some();
    let server_tool_signal = supplied
        .server_tool_use
        .and_then(|server_tool_use| server_tool_use.web_search_requests)
        .is_some();
    prompt_signal
        || server_tool_signal
        || supplied
            .server_side_details
            .as_ref()
            .is_some_and(positive_web_search_calls)
}

fn positive_web_search_calls(details: &IndexMap<String, Json>) -> bool {
    match details.get("web_search_calls") {
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => integer_above_zero(number),
        None | Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) => false,
    }
}

fn integer_above_zero(number: &Number) -> bool {
    if let Some(value) = number.as_i64() {
        return value > 0;
    }
    let literal = number.to_string();
    if literal.contains(['.', 'e', 'E']) {
        return false;
    }
    !literal.starts_with('-')
}

mod tests;
