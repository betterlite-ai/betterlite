use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::OpenAiError;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChatBase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResponsesBase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ImageEditBase;

pub fn provider_config<Config>(config: &Config) -> Result<Map<String, Value>, OpenAiError>
where
    Config: Serialize,
{
    let serialized =
        serde_json::to_value(config).map_err(|source| OpenAiError::ConfigSerialize { source })?;
    match serialized {
        Value::Object(fields) => Ok(fields
            .into_iter()
            .filter(|(name, value)| !name.starts_with('_') && !value.is_null())
            .collect()),
        Value::Null => Ok(Map::new()),
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(OpenAiError::ConfigNotObject)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThinkingRequest {
    pub thinking: bool,
    pub max_tokens: bool,
}

pub fn thinking_request(params: &Map<String, Value>) -> ThinkingRequest {
    let thinking = params.get("thinking");
    let thinking_type = thinking.and_then(|value| value.as_object()?.get("type"));
    let effort = params.get("reasoning_effort");
    ThinkingRequest {
        thinking: thinking == Some(&Value::Bool(true))
            || thinking_type.and_then(Value::as_str) == Some("enabled")
            || effort.is_some_and(|value| !value.is_null()),
        max_tokens: params.contains_key("max_tokens")
            || params.contains_key("max_completion_tokens"),
    }
}

pub fn add_thinking_max_tokens(
    requested: &Map<String, Value>,
    options: &mut Map<String, Value>,
    default_max_tokens: i64,
) -> Result<(), OpenAiError> {
    if !thinking_request(options).thinking || thinking_request(requested).max_tokens {
        return Ok(());
    }
    let budget = options
        .get("thinking")
        .and_then(|thinking| thinking.as_object()?.get("budget_tokens"))
        .filter(|budget| !budget.is_null());
    let Some(tokens) = budget else {
        return Ok(());
    };
    let max_tokens = tokens
        .as_i64()
        .ok_or(OpenAiError::BudgetNotInteger)?
        .checked_add(default_max_tokens)
        .ok_or(OpenAiError::BudgetOverflow)?;
    options.insert("max_tokens".to_owned(), Value::from(max_tokens));
    Ok(())
}

pub fn append_tools(options: &mut Map<String, Value>, tools: Vec<Value>) -> Result<(), OpenAiError> {
    match options.get_mut("tools") {
        None => {
            options.insert("tools".to_owned(), Value::Array(tools));
        }
        Some(Value::Array(existing)) => existing.extend(tools),
        Some(Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_)) => {
            return Err(OpenAiError::ToolsNotList);
        }
    }
    Ok(())
}

pub fn response_format_refs(
    format: Option<&Map<String, Value>>,
    ref_template: Option<&str>,
) -> Result<Option<Map<String, Value>>, OpenAiError> {
    let Some(response_format) = format else {
        return Ok(None);
    };
    let json_schema = response_format.get("type").and_then(Value::as_str) == Some("json_schema");
    let Some(template) = ref_template.filter(|_| json_schema) else {
        return Ok(Some(response_format.clone()));
    };
    let mut rewritten = response_format.clone();
    let envelope = rewritten
        .get_mut("json_schema")
        .ok_or(OpenAiError::MissingKey { key: "json_schema" })?;
    let Value::Object(envelope_fields) = envelope else {
        return Err(OpenAiError::NotSubscriptable { key: "json_schema" });
    };
    let schema = envelope_fields
        .get_mut("schema")
        .ok_or(OpenAiError::MissingKey { key: "schema" })?;
    let mut pending = vec![schema];
    while let Some(node) = pending.pop() {
        match node {
            Value::Object(fields) => {
                if let Some(reference) = fields.get_mut("$ref") {
                    let Value::String(path) = reference else {
                        return Err(OpenAiError::RefNotText);
                    };
                    let model = path.rsplit_once('/').map_or(path.as_str(), |(_, name)| name);
                    *path = format_ref(template, model)?;
                }
                pending.extend(fields.values_mut());
            }
            Value::Array(items) => pending.extend(items.iter_mut()),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    Ok(Some(rewritten))
}

fn format_ref(template: &str, model: &str) -> Result<String, OpenAiError> {
    let mut formatted = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(position) = rest.find(['{', '}']) {
        let (literal, marker) = rest.split_at(position);
        formatted.push_str(literal);
        if let Some(after) = marker.strip_prefix("{{").or_else(|| marker.strip_prefix("}}")) {
            formatted.push_str(marker.get(..1).unwrap_or_default());
            rest = after;
            continue;
        }
        let field_text = marker.strip_prefix('{').ok_or(OpenAiError::TemplateBrace)?;
        let (field, after) = field_text.split_once('}').ok_or(OpenAiError::TemplateBrace)?;
        match field {
            "model" => formatted.push_str(model),
            positional if positional.is_empty() || positional.starts_with(|first: char| first.is_ascii_digit()) => {
                return Err(OpenAiError::TemplatePosition);
            }
            named => {
                return Err(OpenAiError::TemplateField {
                    field: named.to_owned(),
                });
            }
        }
        rest = after;
    }
    formatted.push_str(rest);
    Ok(formatted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    Dict,
    Object,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenCarrier {
    pub kind: ResponseKind,
    // None covers a missing value and one that is neither a dict nor a model, since both read as {}
    pub hidden_params: Option<Map<String, Value>>,
}

pub fn hidden_params(response: &mut HiddenCarrier, create: bool) -> Map<String, Value> {
    let current = response.hidden_params.clone().unwrap_or_default();
    if current.is_empty() && create && response.kind == ResponseKind::Dict {
        response.hidden_params = Some(Map::new());
    }
    current
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct HistoryMessage {
    pub role: Option<String>,
    pub tool_calls: Option<Value>,
    pub thinking_blocks: Option<Value>,
}

fn block_count(blocks: &Value) -> Option<usize> {
    match blocks {
        Value::Array(items) => Some(items.len()),
        Value::Object(fields) => Some(fields.len()),
        Value::String(text) => Some(text.chars().count()),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

pub fn has_tool_calls(messages: &[HistoryMessage]) -> bool {
    for message in messages {
        if message.tool_calls.is_some() {
            return true;
        }
    }
    false
}

pub fn assistant_has_thinking(messages: &[HistoryMessage]) -> bool {
    for message in messages {
        let blocks = message.thinking_blocks.as_ref();
        let assistant = message.role.as_deref() == Some("assistant");
        if assistant && blocks.is_some_and(|value| block_count(value).is_none_or(|count| count > 0)) {
            return true;
        }
    }
    false
}

pub fn last_tool_call_unthought(messages: &[HistoryMessage]) -> bool {
    let mut last_with_tools = None;
    for message in messages {
        if message.role.as_deref() == Some("assistant") && message.tool_calls.is_some() {
            last_with_tools = Some(message);
        }
    }
    last_with_tools.is_some_and(|message| {
        let blocks = message.thinking_blocks.as_ref();
        blocks.is_none_or(|value| block_count(value) == Some(0))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: ObjectParameters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectParameters {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub properties: BTreeMap<&'static str, ObjectParameters>,
}

pub const DUMMY_TOOLS: [ToolDefinition; 1] = [ToolDefinition {
    kind: "function",
    function: FunctionDefinition {
        name: "dummy_tool",
        description: "This is a dummy tool call",
        parameters: ObjectParameters {
            kind: "object",
            properties: BTreeMap::new(),
        },
    },
}];
