use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::sanitize::{AnthropicInputSchema, sanitize_for_anthropic};

use super::ToolsError;
use super::fields::{
    function_fields, non_null, optional_number, optional_string, required_number, required_string,
};
use super::options::{collect_options, insert_option};
use super::wire::{
    ADVISOR_TYPE, AdvisorTool, CUSTOM_TYPE, ComputerTool, FunctionTool, HostedTool, McpServerTool,
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

pub(super) fn function_tool(tool: &Json, fields: &Object) -> Result<ToolDefinition, ToolsError> {
    let function = function_fields(fields)?.ok_or(ToolsError::Missing("function"))?;
    let schema = match non_null(function.get("parameters")) {
        None => Object::new(),
        Some(Json::Object(parameters)) => parameters.clone(),
        Some(_) => {
            return Err(ToolsError::WrongType {
                field: "parameters",
                expected: "an object",
            });
        }
    };
    let input_schema = schema_value(sanitize_for_anthropic(schema)?);
    let eager_input_streaming = eager_input_streaming_flag(tool)?;
    let name = required_string(function, "name")?;
    let options = collect_options(fields, CUSTOM_TYPE)?;
    Ok(ToolDefinition::Function(FunctionTool {
        name: name.to_owned(),
        input_schema,
        kind: CUSTOM_TYPE,
        eager_input_streaming,
        description: optional_string(function, "description")?.map(str::to_owned),
        cache_control: options.cache_control,
        defer_loading: options.defer_loading,
        allowed_callers: options.allowed_callers,
        input_examples: options.input_examples,
    }))
}

pub(super) fn computer_tool(kind: &str, fields: &Object) -> Result<ToolDefinition, ToolsError> {
    let function = function_fields(fields)?.ok_or(ToolsError::Missing("function"))?;
    let declared = non_null(function.get("parameters")).ok_or(ToolsError::Missing("parameters"))?;
    let parameters = declared.as_object().ok_or(ToolsError::WrongType {
        field: "parameters",
        expected: "an object",
    })?;
    let display_width_px = required_number(parameters, "display_width_px")?.clone();
    let display_height_px = required_number(parameters, "display_height_px")?.clone();
    let name = optional_string(function, "name")?.unwrap_or("computer");
    let options = collect_options(fields, kind)?;
    Ok(ToolDefinition::Computer(ComputerTool {
        kind: kind.to_owned(),
        name: name.to_owned(),
        display_width_px,
        display_height_px,
        display_number: optional_number(parameters, "display_number")?.cloned(),
        cache_control: options.cache_control,
        defer_loading: options.defer_loading,
        allowed_callers: options.allowed_callers,
    }))
}

pub(super) fn hosted_tool(kind: &str, fields: &Object) -> Result<ToolDefinition, ToolsError> {
    let name = hosted_name(fields)?;
    let mut extra = fields.clone();
    extra.shift_remove("type");
    extra.shift_remove("name");
    let options = collect_options(fields, kind)?;
    insert_option(&mut extra, "cache_control", options.cache_control);
    insert_option(
        &mut extra,
        "defer_loading",
        options.defer_loading.map(Json::Bool),
    );
    insert_option(
        &mut extra,
        "allowed_callers",
        options
            .allowed_callers
            .map(|callers| Json::List(callers.into_iter().map(Json::Text).collect())),
    );
    Ok(ToolDefinition::Hosted(HostedTool {
        kind: kind.to_owned(),
        name,
        fields: extra,
    }))
}

fn hosted_name(fields: &Object) -> Result<String, ToolsError> {
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

pub(super) fn advisor_tool(fields: &Object) -> Result<ToolDefinition, ToolsError> {
    let model = fields
        .get("model")
        .and_then(Json::as_str)
        .ok_or(ToolsError::AdvisorModel)?;
    let options = collect_options(fields, ADVISOR_TYPE)?;
    Ok(ToolDefinition::Advisor(AdvisorTool {
        kind: ADVISOR_TYPE,
        name: "advisor",
        model: model.to_owned(),
        max_uses: non_null(fields.get("max_uses")).cloned(),
        caching: non_null(fields.get("caching")).cloned(),
        cache_control: options.cache_control,
        defer_loading: options.defer_loading,
        allowed_callers: options.allowed_callers,
    }))
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
