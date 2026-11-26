use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_request_models::is_falsey;

pub const TOOL_CAPABLE_CALL_TYPES: [&str; 8] = [
    "completion",
    "acompletion",
    "responses",
    "aresponses",
    "anthropic_messages",
    "generate_content",
    "agenerate_content",
    "call_mcp_tool",
];

pub trait ToolRouteCallTypes: fmt::Debug + Send + Sync {
    fn call_types_for_route(&self, route: &str) -> Vec<String>;
}

#[derive(Clone, Copy, Debug)]
pub enum ToolNameExtractor {
    AnthropicBody,
    Chat,
    Gemini,
    Mcp,
    Responses,
}

#[derive(Clone, Debug)]
pub struct ToolNameExtractors {
    pub routes: Arc<dyn ToolRouteCallTypes>,
    // both maps key the CallTypes source names, such as completion or anthropic_messages
    pub standalone: BTreeMap<&'static str, ToolNameExtractor>,
    pub handlers: BTreeMap<&'static str, ToolNameExtractor>,
}

impl ToolNameExtractors {
    #[must_use]
    pub fn builtin(routes: Arc<dyn ToolRouteCallTypes>) -> Self {
        let mut extractors = Self {
            routes,
            standalone: BTreeMap::new(),
            handlers: BTreeMap::new(),
        };
        extractors.register_standalone_builtins();
        extractors.register_handler_builtins();
        extractors
    }

    // an existing registry is left alone, like the module's one-time Python registration
    pub fn register_standalone_builtins(&mut self) {
        if !self.standalone.is_empty() {
            return;
        }
        for route in ["generate_content", "agenerate_content"] {
            self.standalone.insert(route, ToolNameExtractor::Gemini);
        }
        self.standalone
            .insert("call_mcp_tool", ToolNameExtractor::Mcp);
    }

    fn register_handler_builtins(&mut self) {
        self.handlers.insert("completion", ToolNameExtractor::Chat);
        self.handlers.insert("acompletion", ToolNameExtractor::Chat);
        self.handlers
            .insert("responses", ToolNameExtractor::Responses);
        self.handlers
            .insert("aresponses", ToolNameExtractor::Responses);
        self.handlers
            .insert("anthropic_messages", ToolNameExtractor::AnthropicBody);
    }
}

pub fn extract_request_tool_names(
    route: &str,
    data: &JsonObject,
    extractors: &ToolNameExtractors,
) -> Result<Vec<String>, ExtractionError> {
    for source_name in extractors.routes.call_types_for_route(route) {
        if !TOOL_CAPABLE_CALL_TYPES.contains(&source_name.as_str()) {
            continue;
        }
        let standalone = extractors.standalone.get(source_name.as_str());
        let handler = extractors.handlers.get(source_name.as_str());
        let Some(extractor) = standalone.or(handler) else {
            continue;
        };
        let names = match *extractor {
            ToolNameExtractor::AnthropicBody => anthropic_body_tool_names(data),
            ToolNameExtractor::Chat => chat_tool_names(data),
            ToolNameExtractor::Gemini => gemini_tool_names(data),
            ToolNameExtractor::Mcp => Ok(mcp_tool_names(data)),
            ToolNameExtractor::Responses => responses_tool_names(data),
        }?;
        if !names.is_empty() || standalone.is_some() {
            return Ok(names);
        }
    }
    Ok(Vec::new())
}

#[derive(Debug, thiserror::Error)]
pub enum ExtractionError {
    #[error("the request's {field} value is not iterable")]
    NotIterable { field: &'static str },
}

impl ExtractionError {
    #[must_use]
    pub fn into_auth_failure(self) -> AuthFailure {
        AuthFailure::Other {
            message: self.to_string(),
            param: None,
        }
    }

    #[must_use]
    pub fn into_proxy_exception(self) -> ProxyException {
        as_proxy_exception(&self.into_auth_failure())
            .exception()
            .clone()
    }
}

// data.get(field) or []: an absent or falsey value iterates nothing, a string or object iterates non-dicts, and a truthy scalar raises
fn iterable_values<'json>(
    data: &'json JsonObject,
    field: &'static str,
) -> Result<&'json [Json], ExtractionError> {
    let Some(value) = data.get(field) else {
        return Ok(&[]);
    };
    if is_falsey(value) {
        return Ok(&[]);
    }
    match value {
        Json::List(items) => Ok(items),
        Json::Text(_) | Json::Object(_) | Json::Null | Json::Bool(false) => Ok(&[]),
        Json::Bool(true) | Json::Number(_) => Err(ExtractionError::NotIterable { field }),
    }
}

fn type_name(tool: &JsonObject) -> Option<&str> {
    match tool.get("type") {
        Some(Json::Text(name)) => Some(name.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn source_text(value: Option<&Json>) -> Option<String> {
    let present = value?;
    if is_falsey(present) {
        return None;
    }
    Some(source_string(present))
}

fn traverse<'json>(
    values: &'json [Json],
    pick: fn(&'json JsonObject) -> Option<String>,
) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| match value {
            Json::Object(tool) => pick(tool),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn source_string(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) | Json::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn push_nonempty_text(names: &mut Vec<String>, value: Option<&Json>) {
    if let Some(Json::Text(text)) = value
        && !text.is_empty()
    {
        names.push(text.clone());
    }
}

fn chat_tool_name(tool: &JsonObject) -> Option<String> {
    if type_name(tool) != Some("function") {
        return None;
    }
    match tool.get("function") {
        Some(Json::Object(function)) => source_text(function.get("name")),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

pub(crate) fn chat_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = traverse(iterable_values(data, "tools")?, chat_tool_name);
    names.extend(traverse(iterable_values(data, "functions")?, |legacy| {
        source_text(legacy.get("name"))
    }));
    Ok(names)
}

fn responses_tool_name(tool: &JsonObject) -> Option<String> {
    match type_name(tool) {
        Some("function" | "custom") => source_text(tool.get("name")),
        Some("mcp") => source_text(tool.get("server_label")),
        Some(_) | None => None,
    }
}

pub(crate) fn responses_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    Ok(traverse(
        iterable_values(data, "tools")?,
        responses_tool_name,
    ))
}

pub(crate) fn anthropic_body_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = Vec::new();
    for tool in iterable_values(data, "tools")? {
        names.extend(anthropic_tool_names(tool));
    }
    Ok(names)
}

#[must_use]
pub fn anthropic_tool_names(tool: &Json) -> Vec<String> {
    let Json::Object(fields) = tool else {
        return Vec::new();
    };
    let mut names = Vec::new();
    push_nonempty_text(&mut names, fields.get("name"));
    if type_name(fields) == Some("function")
        && let Some(Json::Object(function)) = fields.get("function")
    {
        push_nonempty_text(&mut names, function.get("name"));
    }
    names
}

fn gemini_entry_names(entry: &Json) -> Result<Vec<String>, ExtractionError> {
    let Json::Object(tool) = entry else {
        return Ok(Vec::new());
    };
    Ok(traverse(
        iterable_values(tool, "functionDeclarations")?,
        |declaration| source_text(declaration.get("name")),
    ))
}

pub(crate) fn gemini_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = Vec::new();
    for entry in iterable_values(data, "tools")? {
        names.extend(gemini_entry_names(entry)?);
    }
    Ok(names)
}

fn mcp_name(data: &JsonObject) -> Option<&Json> {
    let preferred = data.get("name");
    if preferred.is_some_and(|value| !is_falsey(value)) {
        return preferred;
    }
    data.get("mcp_tool_name")
}

fn mcp_tool_names(data: &JsonObject) -> Vec<String> {
    let Some(value) = mcp_name(data).filter(|value| !is_falsey(value)) else {
        return Vec::new();
    };
    vec![source_string(value)]
}
