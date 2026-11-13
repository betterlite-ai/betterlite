use betterlite_models::decisions::Json;

use super::records::ToolDefinition;
use super::values::{Object, as_object, is_filled, mapping_field, text_field};

#[must_use]
pub fn to_dd_tool_definitions(model_parameters: Option<&Json>) -> Vec<ToolDefinition> {
    let Some(Json::Object(parameters)) = model_parameters else {
        return Vec::new();
    };
    let raw_tools = match parameters.get("tools") {
        Some(tools) if is_filled(tools) => tools,
        Some(_) | None => match parameters.get("functions") {
            Some(functions) => functions,
            None => return Vec::new(),
        },
    };
    let Json::List(entries) = raw_tools else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(as_object)
        .filter_map(to_dd_tool_definition)
        .collect()
}

#[must_use]
pub fn to_dd_tool_definition(entry: &Object) -> Option<ToolDefinition> {
    let declared = mapping_field(entry, "function").unwrap_or(entry);
    let name = text_field(declared, "name", "");
    if name.is_empty() {
        return None;
    }
    let description = text_field(declared, "description", "");
    let schema = match declared.get("parameters") {
        Some(parameters) if is_filled(parameters) => Some(parameters),
        Some(_) | None => declared.get("input_schema"),
    };
    Some(ToolDefinition {
        name,
        description,
        schema: schema.and_then(as_object).cloned(),
    })
}
