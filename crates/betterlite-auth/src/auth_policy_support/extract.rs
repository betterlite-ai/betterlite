use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use betterlite_error::proxy_exception::ProxyException;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::auth_errors::conversion::{AuthFailure, as_proxy_exception};
use crate::auth_request_models::is_falsey;

pub type ToolNameExtractor = fn(&JsonObject) -> Result<Vec<String>, ExtractionError>;

// the eight call types whose requests can carry tools, named as LiteLLM's CallTypes values are
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ToolCapableCallType {
    Completion,
    AsyncCompletion,
    Responses,
    AsyncResponses,
    AnthropicMessages,
    GenerateContent,
    AsyncGenerateContent,
    CallMcpTool,
}

impl ToolCapableCallType {
    pub fn from_source_name(name: &str) -> Option<Self> {
        match name {
            "completion" => Some(Self::Completion),
            "acompletion" => Some(Self::AsyncCompletion),
            "responses" => Some(Self::Responses),
            "aresponses" => Some(Self::AsyncResponses),
            "anthropic_messages" => Some(Self::AnthropicMessages),
            "generate_content" => Some(Self::GenerateContent),
            "agenerate_content" => Some(Self::AsyncGenerateContent),
            "call_mcp_tool" => Some(Self::CallMcpTool),
            _ => None,
        }
    }
}

// get_call_types_for_route behind a trait: B207's catalog answers in the server, a fixture answers in a test
pub trait ToolRouteCallTypes: fmt::Debug + Send + Sync {
    fn call_types_for_route(&self, route: &str) -> Vec<String>;
}

#[derive(Clone, Debug)]
pub struct ToolNameExtractors {
    pub routes: Arc<dyn ToolRouteCallTypes>,
    pub standalone: BTreeMap<ToolCapableCallType, ToolNameExtractor>,
    pub handlers: BTreeMap<ToolCapableCallType, ToolNameExtractor>,
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
        self.standalone
            .insert(ToolCapableCallType::GenerateContent, gemini_tool_names);
        self.standalone
            .insert(ToolCapableCallType::AsyncGenerateContent, gemini_tool_names);
        self.standalone
            .insert(ToolCapableCallType::CallMcpTool, mcp_tool_names);
    }

    fn register_handler_builtins(&mut self) {
        self.handlers
            .insert(ToolCapableCallType::Completion, chat_tool_names);
        self.handlers
            .insert(ToolCapableCallType::AsyncCompletion, chat_tool_names);
        self.handlers
            .insert(ToolCapableCallType::Responses, responses_tool_names);
        self.handlers
            .insert(ToolCapableCallType::AsyncResponses, responses_tool_names);
        self.handlers.insert(
            ToolCapableCallType::AnthropicMessages,
            anthropic_body_tool_names,
        );
    }
}

// extract_request_tool_names: a standalone extractor answers alone, and handlers are tried in order until one names a tool
pub fn extract_request_tool_names(
    route: &str,
    data: &JsonObject,
    extractors: &ToolNameExtractors,
) -> Result<Vec<String>, ExtractionError> {
    for source_name in extractors.routes.call_types_for_route(route) {
        let Some(call_type) = ToolCapableCallType::from_source_name(&source_name) else {
            continue;
        };
        if let Some(extractor) = extractors.standalone.get(&call_type) {
            return extractor(data);
        }
        if let Some(handler) = extractors.handlers.get(&call_type) {
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
        let failure = self.into_auth_failure();
        as_proxy_exception(&failure).exception().clone()
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

fn append_truthy_source_string(names: &mut Vec<String>, value: Option<&Json>) {
    let Some(value) = value else {
        return;
    };
    if is_falsey(value) {
        return;
    }
    names.push(source_string(value));
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

// the chat handler's tools[].function.name, then the legacy functions[].name list
pub(crate) fn chat_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = Vec::new();
    for entry in iterable_values(data, "tools")? {
        let Json::Object(tool) = entry else {
            continue;
        };
        if type_name(tool) != Some("function") {
            continue;
        }
        let Some(Json::Object(function)) = tool.get("function") else {
            continue;
        };
        append_truthy_source_string(&mut names, function.get("name"));
    }
    for entry in iterable_values(data, "functions")? {
        let Json::Object(function) = entry else {
            continue;
        };
        append_truthy_source_string(&mut names, function.get("name"));
    }
    Ok(names)
}

// the responses handler's function/custom name and mcp server_label
pub(crate) fn responses_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = Vec::new();
    for entry in iterable_values(data, "tools")? {
        let Json::Object(tool) = entry else {
            continue;
        };
        match type_name(tool) {
            Some("function" | "custom") => {
                append_truthy_source_string(&mut names, tool.get("name"));
            }
            Some("mcp") => {
                append_truthy_source_string(&mut names, tool.get("server_label"));
            }
            Some(_) | None => {}
        }
    }
    Ok(names)
}

// the anthropic handler flattens every tool's name pair
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

// generateContent: tools[].functionDeclarations[].name
pub(crate) fn gemini_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let mut names = Vec::new();
    for entry in iterable_values(data, "tools")? {
        let Json::Object(tool) = entry else {
            continue;
        };
        for declaration in iterable_values(tool, "functionDeclarations")? {
            let Json::Object(fields) = declaration else {
                continue;
            };
            append_truthy_source_string(&mut names, fields.get("name"));
        }
    }
    Ok(names)
}

// MCP call_tool: the truthy name, else the truthy mcp_tool_name
pub(crate) fn mcp_tool_names(data: &JsonObject) -> Result<Vec<String>, ExtractionError> {
    let preferred = data.get("name");
    let name = if preferred.is_some_and(|value| !is_falsey(value)) {
        preferred
    } else {
        data.get("mcp_tool_name")
    };
    if let Some(value) = name
        && !is_falsey(value)
    {
        return Ok(vec![source_string(value)]);
    }
    Ok(Vec::new())
}
