use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    error::McpGuardrailError,
    input::{
        Guardrail, GuardrailInput, MAX_STRUCTURED_CONTENT_SCAN_DEPTH, replace_leaves, string_leaves,
    },
};

pub type JsonPath = Vec<String>;

impl super::McpGuardrailHandler {
    pub async fn process_output_response<G: Guardrail>(
        &self,
        response: &mut Value,
        guardrail: &G,
    ) -> Result<(), McpGuardrailError> {
        let content = response
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut text_paths = Vec::new();
        for (index, item) in content.iter().enumerate() {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                text_paths.push((vec![index.to_string(), "text".to_owned()], text.to_owned()));
            }
        }
        let structured = response.get("structuredContent").cloned();
        let structured_leaves = structured
            .as_ref()
            .map(string_leaves)
            .transpose()?
            .unwrap_or_default();
        let labels = structured
            .as_ref()
            .map(unrewritable_labels)
            .transpose()?
            .unwrap_or_default();
        if text_paths.is_empty() && structured_leaves.is_empty() && labels.is_empty() {
            return Ok(());
        }
        let mut originals: Vec<String> =
            text_paths.iter().map(|(_, value)| value.clone()).collect();
        originals.extend(structured_leaves.iter().map(|(_, value)| value.clone()));
        originals.extend(labels.iter().map(|(_, value)| value.clone()));
        let output = guardrail
            .apply(GuardrailInput {
                tool: None,
                texts: originals.clone(),
            })
            .await
            .map_err(McpGuardrailError::Guardrail)?;
        let Some(masked) = output.texts else {
            return Ok(());
        };
        if masked.len() != originals.len() {
            return Err(McpGuardrailError::TextCount {
                returned: masked.len(),
                expected: originals.len(),
            });
        }
        if labels.iter().enumerate().any(|(index, (_, original))| {
            masked[text_paths.len() + structured_leaves.len() + index] != *original
        }) {
            return Err(McpGuardrailError::Blocked {
                reason: "MCP result matched a non-rewritable structured label".to_owned(),
            });
        }
        if let Some(values) = response.get_mut("content").and_then(Value::as_array_mut) {
            for (masked_index, (path, original)) in text_paths.iter().enumerate() {
                if masked[masked_index] != *original {
                    let Some(content_index) =
                        path.first().and_then(|value| value.parse::<usize>().ok())
                    else {
                        continue;
                    };
                    if let Some(text) = values
                        .get_mut(content_index)
                        .and_then(|item| item.get_mut("text"))
                    {
                        *text = Value::String(masked[masked_index].clone());
                    }
                }
            }
        }
        let replacements: BTreeMap<_, _> = structured_leaves
            .iter()
            .zip(&masked[text_paths.len()..text_paths.len() + structured_leaves.len()])
            .filter(|((_, original), replacement)| original != *replacement)
            .map(|((path, _), replacement)| (path.clone(), replacement.clone()))
            .collect();
        if let Some(value) = structured {
            if !replacements.is_empty() {
                response.as_object_mut().and_then(|object| {
                    object.insert(
                        "structuredContent".to_owned(),
                        replace_leaves(value, &replacements),
                    )
                });
            }
        }
        Ok(())
    }
}

fn unrewritable_labels(value: &Value) -> Result<Vec<(JsonPath, String)>, McpGuardrailError> {
    let mut labels = Vec::new();
    collect_labels(value, &mut Vec::new(), &mut labels, 0)?;
    Ok(labels)
}

fn collect_labels(
    value: &Value,
    path: &mut JsonPath,
    labels: &mut Vec<(JsonPath, String)>,
    depth: usize,
) -> Result<(), McpGuardrailError> {
    if depth > MAX_STRUCTURED_CONTENT_SCAN_DEPTH {
        return Err(McpGuardrailError::Blocked {
            reason: "MCP structured content exceeds the maximum nesting depth".to_owned(),
        });
    }
    match value {
        Value::Number(number) => labels.push((path.clone(), number.to_string())),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                path.push(index.to_string());
                collect_labels(value, path, labels, depth + 1)?;
                path.pop();
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                labels.push((path.clone(), key.clone()));
                path.push(key.clone());
                collect_labels(value, path, labels, depth + 1)?;
                path.pop();
            }
        }
        Value::String(_) | Value::Bool(_) | Value::Null => {}
    }
    Ok(())
}
