use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::RequestToolsError;
use crate::tool_permission::types::{mapping_value, object_mapping};

type Data = BTreeMap<String, GuardrailValue>;
type NameReader = fn(&GuardrailValue, &str) -> Result<Option<String>, RequestToolsError>;

// Each slot keeps the position of its entry in the request list, so a rewrite drops the same entries; a tool has a name only when it is a function tool
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestTools {
    pub tools: Option<Vec<Option<String>>>,
    pub functions: Option<Vec<Option<String>>>,
    pub tool_choice: Option<String>,
    pub function_call: Option<String>,
}

impl RequestTools {
    // LiteLLM's _collect_request_tools: every name is checked as a function tool, in this order
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .chain(self.functions.iter())
            .flatten()
            .flatten()
            .chain(self.tool_choice.iter())
            .chain(self.function_call.iter())
            .map(String::as_str)
            .collect()
    }
}

pub fn extract_request_tools(data: &Data) -> Result<RequestTools, RequestToolsError> {
    Ok(RequestTools {
        tools: list_names(data, "tools", tool_name)?,
        functions: list_names(data, "functions", function_name)?,
        tool_choice: named_tool_choice(data)?,
        function_call: named_function_call(data)?,
    })
}

fn list_names(
    data: &Data,
    field: &str,
    name_of: NameReader,
) -> Result<Option<Vec<Option<String>>>, RequestToolsError> {
    let items = match data.get(field) {
        None | Some(GuardrailValue::Null) => return Ok(None),
        Some(GuardrailValue::Array(items)) => items,
        Some(_) => return Err(RequestToolsError::WrongType(field.to_owned(), "a list")),
    };
    let mut names = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        names.push(name_of(item, &format!("{field}[{index}]"))?);
    }
    Ok(Some(names))
}

fn text_field<'value>(
    value: &'value GuardrailValue,
    key: &str,
    path: &str,
) -> Result<Option<&'value str>, RequestToolsError> {
    match mapping_value(value, key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text)),
        Some(_) => Err(RequestToolsError::WrongType(
            format!("{path}.{key}"),
            "a string",
        )),
    }
}

fn object_field<'value>(
    value: &'value GuardrailValue,
    key: &str,
    path: &str,
) -> Result<Option<&'value GuardrailValue>, RequestToolsError> {
    match mapping_value(value, key) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(object @ GuardrailValue::Object(_)) => Ok(Some(object)),
        Some(_) => Err(RequestToolsError::WrongType(
            format!("{path}.{key}"),
            "an object",
        )),
    }
}

// LiteLLM's _get_request_tool_name: only a function tool carries a name
fn tool_name(tool: &GuardrailValue, path: &str) -> Result<Option<String>, RequestToolsError> {
    if object_mapping(tool).is_none() {
        return Err(RequestToolsError::WrongType(path.to_owned(), "an object"));
    }
    if text_field(tool, "type", path)? != Some("function") {
        return Ok(None);
    }
    let Some(function) = object_field(tool, "function", path)? else {
        return Ok(None);
    };
    Ok(text_field(function, "name", &format!("{path}.function"))?.map(str::to_owned))
}

// LiteLLM's _get_legacy_function_name
fn function_name(
    function: &GuardrailValue,
    path: &str,
) -> Result<Option<String>, RequestToolsError> {
    if object_mapping(function).is_none() {
        return Err(RequestToolsError::WrongType(path.to_owned(), "an object"));
    }
    Ok(text_field(function, "name", path)?.map(str::to_owned))
}

// LiteLLM's _get_named_tool_choice: auto, none and required force nothing
fn named_tool_choice(data: &Data) -> Result<Option<String>, RequestToolsError> {
    match data.get("tool_choice") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(choice)) => {
            let forces_nothing = matches!(choice.as_str(), "auto" | "none" | "required");
            Ok((!forces_nothing).then(|| choice.clone()))
        }
        Some(choice @ GuardrailValue::Object(_)) => {
            if text_field(choice, "type", "tool_choice")? != Some("function") {
                return Ok(None);
            }
            let Some(function) = object_field(choice, "function", "tool_choice")? else {
                return Ok(None);
            };
            Ok(text_field(function, "name", "tool_choice.function")?.map(str::to_owned))
        }
        Some(_) => Err(RequestToolsError::WrongType(
            "tool_choice".to_owned(),
            "a string or an object",
        )),
    }
}

// LiteLLM's _get_named_function_call: auto and none force nothing
fn named_function_call(data: &Data) -> Result<Option<String>, RequestToolsError> {
    match data.get("function_call") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(call)) => {
            let forces_nothing = matches!(call.as_str(), "auto" | "none");
            Ok((!forces_nothing).then(|| call.clone()))
        }
        Some(call @ GuardrailValue::Object(_)) => {
            Ok(text_field(call, "name", "function_call")?.map(str::to_owned))
        }
        Some(_) => Err(RequestToolsError::WrongType(
            "function_call".to_owned(),
            "a string or an object",
        )),
    }
}

mod tests;
