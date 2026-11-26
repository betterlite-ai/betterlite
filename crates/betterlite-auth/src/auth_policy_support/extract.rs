use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_request_models::is_falsey;

pub type ToolNameExtractor = fn(&JsonObject) -> Result<Vec<String>, ExtractionError>;

// get_call_types_for_route behind a trait: B207's catalog answers in the server, a fixture answers in a test
pub trait ToolRouteCallTypes: fmt::Debug + Send + Sync {
    fn call_types_for_route(&self, route: &str) -> Vec<String>;
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
            self.standalone.insert(route, gemini_tool_names);
        }
        self.standalone.insert("call_mcp_tool", mcp_tool_names);
    }

    fn register_handler_builtins(&mut self) {
        self.handlers.insert("completion", chat_tool_names);
        self.handlers.insert("acompletion", chat_tool_names);
        self.handlers.insert("responses", responses_tool_names);
        self.handlers.insert("aresponses", responses_tool_names);
        self.handlers
            .insert("anthropic_messages", anthropic_body_tool_names);
    }
}

// extract_request_tool_names: a standalone extractor answers alone, and handlers are tried in order until one names a tool
pub fn extract_request_tool_names(
    route: &str,
    data: &JsonObject,
    extractors: &ToolNameExtractors,
) -> Result<Vec<String>, ExtractionError> {
    for source_name in extractors.routes.call_types_for_route(route) {
        if let Some(extractor) = extractors.standalone.get(source_name.as_str()) {
            return extractor(data);
        }
        if let Some(handler) = extractors.handlers.get(source_name.as_str()) {
            let names = handler(data)?;
            if !names.is_empty() {
                return Ok(names);
            }
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

// the Python truthy check, then the same source string conversion the named values use
fn source_text(value: Option<&Json>) -> Option<String> {
    let value = value?;
    if is_falsey(value) {
        return None;
    }
    Some(source_string(value))
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

// the flat name and, only for type function, the forwarded OpenAI function.name, both nonempty strings
#[must_use]
pub fn anthropic_tool_names(tool: &Json) -> Vec<String> {
    let Json::Object(tool) = tool else {
        return Vec::new();
    };
    let mut names = Vec::new();
    push_nonempty_text(&mut names, tool.get("name"));
    if type_name(tool) == Some("function")
        && let Some(Json::Object(function)) = tool.get("function")
    {
        push_nonempty_text(&mut names, function.get("name"));
    }
    names
}

// one generateContent tool's functionDeclarations names, and nothing for a non-object entry
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

// MCP call_tool: the truthy name, else the mcp_tool_name
fn mcp_name(data: &JsonObject) -> Option<&Json> {
    let preferred = data.get("name");
    if preferred.is_some_and(|value| !is_falsey(value)) {
        return preferred;
    }
    data.get("mcp_tool_name")
}

pub(crate) fn mcp_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let Some(value) = mcp_name(data).filter(|value| !is_falsey(value)) else {
        return Ok(Vec::new());
    };
    Ok(vec![source_string(value)])
}
