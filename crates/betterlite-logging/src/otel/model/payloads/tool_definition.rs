use super::super::utils::coercion::OtelModelValue;
use crate::otel::model::metadata::wire::MetadataMap;

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters_json: Option<String>,
}

#[must_use]
pub fn extract_tools(model_parameters: &MetadataMap) -> Vec<ToolDefinition> {
    let entries = model_parameters
        .get("tools")
        .and_then(OtelModelValue::as_array)
        .or_else(|| {
            model_parameters
                .get("functions")
                .and_then(OtelModelValue::as_array)
        });
    entries.map_or_else(Vec::new, |declared| {
        declared.iter().filter_map(tool_from_entry).collect()
    })
}

fn tool_from_entry(entry: &OtelModelValue) -> Option<ToolDefinition> {
    let entry_fields = entry.as_object()?;
    // a `function` key that exists but holds null is unusable, so the entry itself is no fallback
    let function = entry_fields.get("function").unwrap_or(entry).as_object()?;
    let name = function.get("name").and_then(OtelModelValue::as_str)?;
    if name.is_empty() {
        return None;
    }
    Some(ToolDefinition {
        name: name.to_owned(),
        description: function
            .get("description")
            .and_then(OtelModelValue::as_str)
            .map(str::to_owned),
        parameters_json: function
            .get("parameters")
            .filter(|parameters| !matches!(parameters, OtelModelValue::Null(())))
            .and_then(|parameters| serde_json::to_string(parameters).ok()),
    })
}

#[path = "tool_definition/tests.rs"]
mod tests;
