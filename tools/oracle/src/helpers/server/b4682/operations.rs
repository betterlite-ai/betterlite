use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::{JsonValue, PinnedMCPTool};
use betterlite_server::mcp::catalog_guard::headers::{
    HeaderSanitizeContext, logging_safe_mcp_headers,
};
use betterlite_server::mcp::catalog_guard::pin::pin_tool_catalog;
use betterlite_server::mcp::catalog_guard::prefix::{PrefixPolicy, strip_known_server_prefix};
use betterlite_server::mcp::catalog_guard::types::{MCPServer, MCPTool};
use serde_json::{Value, json};

use super::input::{PinnedInput, ServerInput, ToolInput};

mod scan;

pub(super) use scan::run_scan;

pub(super) fn run_prefix(name: &str, server: Option<&ServerInput>) -> Value {
    let configured_server = server.map(to_server);
    let bare_name =
        strip_known_server_prefix(name, configured_server.as_ref(), &PrefixPolicy::default());
    Value::String(bare_name)
}

pub(super) fn sanitize_headers(
    headers: &BTreeMap<String, String>,
    credentials: &[String],
    identity: &[String],
    client_auth: &str,
    custom_key: Option<&str>,
) -> crate::helpers::HelperResult {
    let context = HeaderSanitizeContext {
        upstream_credential_names: credentials,
        identity_header_names: identity,
        custom_key_header_name: custom_key,
        client_side_auth_header_name: client_auth,
    };
    serde_json::to_value(logging_safe_mcp_headers(headers, &context))
        .map_err(|source| error("SerializationError", source.to_string()))
}

pub(super) fn run_pin(
    inputs: &[ToolInput],
    pin_inputs: &BTreeMap<String, PinnedInput>,
) -> crate::helpers::HelperResult {
    let catalog_tools = convert_tools(inputs)?;
    let (served, drift) = pin_tool_catalog(&catalog_tools, &to_pins(pin_inputs)?);
    Ok(json!({
        "served": served,
        "added": drift.as_ref().map_or_else(Vec::new, |value| value.added.clone()),
        "removed": drift.as_ref().map_or_else(Vec::new, |value| value.removed.clone()),
        "changed": drift.as_ref().map_or_else(Vec::new, |value| value.changed.clone()),
    }))
}

pub(super) fn run_override(
    inputs: &[ToolInput],
    overrides: &BTreeMap<String, String>,
    prefix: Option<&str>,
) -> crate::helpers::HelperResult {
    let served =
        betterlite_server::mcp::catalog_guard::description_overrides::apply_description_overrides(
            &convert_tools(inputs)?,
            &override_server(overrides, prefix),
            &PrefixPolicy::default(),
        );
    serde_json::to_value(served).map_err(|source| error("SerializationError", source.to_string()))
}

fn to_pins(
    pin_inputs: &BTreeMap<String, PinnedInput>,
) -> Result<BTreeMap<String, PinnedMCPTool>, Box<crate::helpers::HelperError>> {
    pin_inputs
        .iter()
        .map(|(name, value)| Ok((name.clone(), to_pinned(value)?)))
        .collect()
}

fn override_server(overrides: &BTreeMap<String, String>, prefix: Option<&str>) -> MCPServer {
    MCPServer {
        server_id: "server-id".to_owned(),
        name: "server".to_owned(),
        alias: prefix.map(str::to_owned),
        server_name: None,
        short_prefix: None,
        extra_headers: Vec::new(),
        tool_name_to_description: overrides.clone(),
    }
}

pub(super) fn convert_tools(
    inputs: &[ToolInput],
) -> Result<Vec<MCPTool>, Box<crate::helpers::HelperError>> {
    let mut tools = Vec::with_capacity(inputs.len());
    for input in inputs {
        tools.push(MCPTool::try_from(input)?);
    }
    Ok(tools)
}

impl TryFrom<&ToolInput> for MCPTool {
    type Error = Box<crate::helpers::HelperError>;

    fn try_from(input: &ToolInput) -> Result<Self, Self::Error> {
        Ok(Self {
            name: input.name.clone(),
            description: input.description.clone(),
            title: input.title.clone(),
            input_schema: json_object(&input.input_schema)?,
            output_schema: input.output_schema.as_ref().map(json_object).transpose()?,
            execution: input.execution.as_ref().map(json_object).transpose()?,
            annotations: input.annotations.as_ref().map(json_object).transpose()?,
            icons: input
                .icons
                .as_ref()
                .map(|values| values.iter().map(json_value).collect())
                .transpose()?,
            meta: input.meta.as_ref().map(json_object).transpose()?,
        })
    }
}

fn to_pinned(input: &PinnedInput) -> Result<PinnedMCPTool, Box<crate::helpers::HelperError>> {
    let input_schema = json_object(&input.input_schema)?;
    let encoded = serde_json::to_string(&json!({
        "description": input.description,
        "input_schema": input_schema,
    }))
    .map_err(|source| error("SerializationError", source.to_string()))?;
    serde_json::from_str(&encoded).map_err(|source| error("InvalidPinnedTool", source.to_string()))
}

pub(super) fn json_object(
    values: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, JsonValue>, Box<crate::helpers::HelperError>> {
    values
        .iter()
        .map(|(name, value)| Ok((name.clone(), json_value(value)?)))
        .collect()
}

fn json_value(value: &Value) -> Result<JsonValue, Box<crate::helpers::HelperError>> {
    let encoded = serde_json::to_string(value)
        .map_err(|source| error("SerializationError", source.to_string()))?;
    serde_json::from_str(&encoded).map_err(|source| error("InvalidJsonValue", source.to_string()))
}

pub(super) fn to_server(input: &ServerInput) -> MCPServer {
    MCPServer {
        server_id: input.server_id.clone(),
        name: input.name.clone(),
        alias: input.alias.clone(),
        server_name: input.server_name.clone(),
        short_prefix: input.short_prefix.clone(),
        extra_headers: Vec::new(),
        tool_name_to_description: BTreeMap::new(),
    }
}

pub(super) fn error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
