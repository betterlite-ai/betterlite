use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::sanitize::{AnthropicInputSchema, sanitize_for_anthropic};

use super::ToolsError;
use super::fields::{field, function_fields, non_null, number};
use super::options::collect_options;
use super::wire::{
    ADVISOR_TYPE, AdvisorTool, CUSTOM_TYPE, ComputerTool, FunctionTool, McpServerTool,
    ToolDefinition,
};
use crate::feature_flags::tools::eager_input_streaming_flag;

pub(super) fn push(
    mapping: &mut super::ToolMapping,
    mapped: (Option<ToolDefinition>, Option<McpServerTool>),
) {
    if let Some(tool) = mapped.0 {
        mapping.tools.push(tool);
    }
    if let Some(server) = mapped.1 {
        mapping.mcp_servers.push(server);
    }
}

pub(super) fn function_tool(tool: &Json, fields: &Object) -> Result<FunctionTool, ToolsError> {
    let function = function_fields(fields)?.ok_or(ToolsError::Missing("function"))?;
    let named =
        field(function, "name", "a string", Json::as_str)?.ok_or(ToolsError::Missing("name"))?;
    Ok(FunctionTool {
        name: named.to_owned(),
        input_schema: function_schema(function)?,
        kind: CUSTOM_TYPE,
        eager_input_streaming: eager_input_streaming_flag(tool)?,
        description: field(function, "description", "a string", Json::as_str)?.map(str::to_owned),
        options: collect_options(fields, CUSTOM_TYPE)?,
    })
}

fn function_schema(function: &Object) -> Result<Json, ToolsError> {
    let declared = match non_null(function.get("parameters")) {
        None => Object::new(),
        Some(Json::Object(parameters)) => parameters.clone(),
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "parameters",
                expected: "an object",
            });
        }
    };
    Ok(schema_value(sanitize_for_anthropic(declared)?))
}

pub(super) fn computer_tool(kind: &str, fields: &Object) -> Result<ComputerTool, ToolsError> {
    let function = function_fields(fields)?.ok_or(ToolsError::Missing("function"))?;
    let declared = non_null(function.get("parameters")).ok_or(ToolsError::Missing("parameters"))?;
    let parameters = declared.as_object().ok_or(ToolsError::WrongType {
        field: "parameters",
        expected: "an object",
    })?;
    let width = field(parameters, "display_width_px", "a number", number)?
        .ok_or(ToolsError::Missing("display_width_px or display_height_px"))?
        .clone();
    let height = field(parameters, "display_height_px", "a number", number)?
        .ok_or(ToolsError::Missing("display_width_px or display_height_px"))?
        .clone();
    let name = field(function, "name", "a string", Json::as_str)?;
    Ok(ComputerTool {
        kind: kind.to_owned(),
        name: name.unwrap_or("computer").to_owned(),
        display_width_px: width,
        display_height_px: height,
        display_number: field(parameters, "display_number", "a number", number)?.cloned(),
        options: collect_options(fields, kind)?,
    })
}

// everything the caller sent except the two keys this mapper rewrites
pub(super) fn sent_fields(fields: &Object) -> Object {
    let mut sent = fields.clone();
    sent.shift_remove("type");
    sent.shift_remove("name");
    sent
}

pub(super) fn hosted_name(fields: &Object) -> Result<String, ToolsError> {
    if let Some(value) = fields.get("name") {
        return value
            .as_str()
            .map(str::to_owned)
            .ok_or(ToolsError::Missing("name"));
    }
    function_fields(fields)?
        .and_then(|function| function.get("name"))
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or(ToolsError::Missing("name"))
}

pub(super) fn advisor_tool(fields: &Object) -> Result<AdvisorTool, ToolsError> {
    match fields.get("model") {
        Some(Json::Text(model)) => Ok(AdvisorTool {
            kind: ADVISOR_TYPE,
            name: "advisor",
            model: model.clone(),
            max_uses: non_null(fields.get("max_uses")).cloned(),
            caching: non_null(fields.get("caching")).cloned(),
            options: collect_options(fields, ADVISOR_TYPE)?,
        }),
        None | Some(_) => Err(ToolsError::AdvisorModel),
    }
}

fn schema_value(schema: AnthropicInputSchema) -> Json {
    let mut fields = Object::new();
    fields.insert("type".to_owned(), schema.schema_type);
    fields.insert("properties".to_owned(), schema.properties);
    if let Some(value) = schema.additional_properties {
        fields.insert("additionalProperties".to_owned(), value);
    }
    if let Some(value) = schema.required {
        fields.insert("required".to_owned(), value);
    }
    if let Some(value) = schema.defs {
        fields.insert("$defs".to_owned(), value);
    }
    if let Some(value) = schema.strict {
        fields.insert("strict".to_owned(), value);
    }
    Json::Object(fields)
}
