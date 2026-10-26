use std::{collections::BTreeMap, future::Future, pin::Pin};

use serde_json::{Map, Value};

use super::{error::McpGuardrailError, output::JsonPath};

pub const MAX_STRUCTURED_CONTENT_SCAN_DEPTH: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailInput {
    pub tool: Option<Value>,
    pub texts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardrailOutput {
    pub texts: Option<Vec<String>>,
}

pub trait Guardrail: Send + Sync {
    fn apply<'a>(
        &'a self,
        input: GuardrailInput,
    ) -> Pin<Box<dyn Future<Output = Result<GuardrailOutput, String>> + Send + 'a>>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct McpGuardrailHandler;

impl McpGuardrailHandler {
    pub async fn process_input_messages<G: Guardrail>(
        &self,
        data: &mut Map<String, Value>,
        guardrail: &G,
    ) -> Result<(), McpGuardrailError> {
        let name = string_field(data, &["mcp_tool_name", "name"]);
        let Some(name) = name.filter(|value| !value.is_empty()) else {
            return Ok(());
        };
        let description = string_field(data, &["mcp_tool_description", "description"]);
        let schema = data
            .get("mcp_input_schema")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let arguments = data
            .get("mcp_arguments")
            .or_else(|| data.get("arguments"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let schema_leaves = description_leaves(&schema)?;
        let argument_leaves = string_leaves(&arguments)?;
        let mut texts = Vec::new();
        if let Some(value) = &description {
            texts.push(value.clone());
        }
        texts.extend(schema_leaves.iter().map(|(_, value)| value.clone()));
        texts.extend(argument_leaves.iter().map(|(_, value)| value.clone()));
        let tool = serde_json::json!({"type":"function","function":{"name":name,"description":description.clone().unwrap_or_default(),"parameters":schema}});
        let output = guardrail
            .apply(GuardrailInput {
                tool: Some(tool),
                texts: texts.clone(),
            })
            .await
            .map_err(McpGuardrailError::Guardrail)?;
        let masked = checked_texts(output.texts, texts.len())?;
        let Some(masked) = masked else { return Ok(()) };
        let description_count = usize::from(description.is_some());
        let schema_end = description_count + schema_leaves.len();
        if let (Some(original), Some(replacement)) = (description.as_ref(), masked.first()) {
            if original != replacement {
                data.insert(
                    "mcp_tool_description".to_owned(),
                    Value::String(replacement.clone()),
                );
            }
        }
        let schema_replacements =
            replacements(&schema_leaves, &masked[description_count..schema_end]);
        if !schema_replacements.is_empty() {
            data.insert(
                "mcp_input_schema".to_owned(),
                replace_leaves(schema, &schema_replacements),
            );
        }
        let argument_replacements = replacements(&argument_leaves, &masked[schema_end..]);
        if argument_replacements.is_empty() {
            return Ok(());
        }
        let current = data
            .get("mcp_arguments")
            .or_else(|| data.get("arguments"))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let current_leaves = string_leaves(&current)?;
        for (path, original) in &argument_leaves {
            if let Some(replacement) = argument_replacements.get(path) {
                let current_value = current_leaves
                    .iter()
                    .find(|(candidate, _)| candidate == path)
                    .map(|(_, value)| value);
                if current_value.is_none()
                    || !matches!(current_value, Some(value) if value == original || value == replacement)
                {
                    return Err(McpGuardrailError::ConflictingRewrite {
                        path: path.join("."),
                    });
                }
            }
        }
        let masked_arguments = replace_leaves(current, &argument_replacements);
        data.insert("mcp_arguments".to_owned(), masked_arguments.clone());
        data.insert("modified_arguments".to_owned(), masked_arguments);
        Ok(())
    }
}

fn string_field(data: &Map<String, Value>, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| data.get(*name).and_then(Value::as_str).map(str::to_owned))
}

fn checked_texts(
    texts: Option<Vec<String>>,
    expected: usize,
) -> Result<Option<Vec<String>>, McpGuardrailError> {
    match texts {
        None => Ok(None),
        Some(values) if values.len() == expected => Ok(Some(values)),
        Some(values) => Err(McpGuardrailError::TextCount {
            returned: values.len(),
            expected,
        }),
    }
}

fn replacements(leaves: &[(JsonPath, String)], masked: &[String]) -> BTreeMap<JsonPath, String> {
    leaves
        .iter()
        .zip(masked)
        .filter(|((_, original), replacement)| original != *replacement)
        .map(|((path, _), replacement)| (path.clone(), replacement.clone()))
        .collect()
}

fn description_leaves(value: &Value) -> Result<Vec<(JsonPath, String)>, McpGuardrailError> {
    Ok(string_leaves(value)?
        .into_iter()
        .filter(|(path, _)| path.last().map(String::as_str) == Some("description"))
        .collect())
}

pub(crate) fn string_leaves(value: &Value) -> Result<Vec<(JsonPath, String)>, McpGuardrailError> {
    let mut leaves = Vec::new();
    collect_strings(value, &mut Vec::new(), &mut leaves, 0)?;
    Ok(leaves)
}

fn collect_strings(
    value: &Value,
    path: &mut JsonPath,
    leaves: &mut Vec<(JsonPath, String)>,
    depth: usize,
) -> Result<(), McpGuardrailError> {
    if depth > MAX_STRUCTURED_CONTENT_SCAN_DEPTH {
        return Err(McpGuardrailError::Blocked {
            reason: "MCP structured content exceeds the maximum nesting depth".to_owned(),
        });
    }
    match value {
        Value::String(text) => leaves.push((path.clone(), text.clone())),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                path.push(index.to_string());
                collect_strings(value, path, leaves, depth + 1)?;
                path.pop();
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                path.push(key.clone());
                collect_strings(value, path, leaves, depth + 1)?;
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

pub(crate) fn replace_leaves(mut value: Value, replacements: &BTreeMap<JsonPath, String>) -> Value {
    for (path, replacement) in replacements {
        if let Some(slot) = value_at_mut(&mut value, path) {
            *slot = Value::String(replacement.clone());
        }
    }
    value
}

fn value_at_mut<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut current = value;
    for part in path {
        current = match current {
            Value::Object(values) => values.get_mut(part)?,
            Value::Array(values) => values.get_mut(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}
