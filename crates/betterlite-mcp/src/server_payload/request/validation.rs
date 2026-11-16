use crate::server_payload::rules::MCP_STDIO_DISABLED_MESSAGE;
use crate::server_payload::{RequestError, quote, quoted_list};
use betterlite_config::sections::mcp_env_values::StdioPolicy;
use betterlite_prompts::arguments::json::{Json, Object};
const DISCOVERY_MESSAGE: &str = "per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream.";
pub(super) fn check(value: &Json, stdio: &StdioPolicy, strict: bool) -> Result<(), RequestError> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    reject_discovery(fields, strict)?;
    check_dcr_bridge(fields, strict)?;
    check_transport(fields, stdio)
}
fn reject_discovery(fields: &Object, strict: bool) -> Result<(), RequestError> {
    if !fields
        .get("per_server_oauth_discovery")
        .is_some_and(Json::is_truthy)
    {
        return Ok(());
    }
    let auth_type_ok = fields.get("auth_type").and_then(Json::as_str) == Some("oauth2")
        || (!strict && !fields.contains_key("auth_type"));
    let flow_ok = fields.get("oauth2_flow").and_then(Json::as_str) == Some("authorization_code")
        || (!strict && !fields.contains_key("oauth2_flow"));
    let delegated = fields
        .get("delegate_auth_to_upstream")
        .is_some_and(Json::is_truthy);
    if auth_type_ok && flow_ok && !delegated {
        return Ok(());
    }
    Err(RequestError::Value(DISCOVERY_MESSAGE.to_owned()))
}
fn check_dcr_bridge(fields: &Object, strict: bool) -> Result<(), RequestError> {
    if !fields.get("dcr_bridge").is_some_and(Json::is_truthy) {
        return Ok(());
    }
    if !strict && !fields.contains_key("auth_type") {
        return Ok(());
    }
    let auth_type = fields.get("auth_type").cloned().unwrap_or(Json::Null);
    if matches!(
        auth_type.as_str(),
        Some("true_passthrough" | "oauth_delegate")
    ) {
        return Ok(());
    }
    Err(RequestError::Value(format!(
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got {}). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow.",
        repr_value(&auth_type)
    )))
}
fn repr_value(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => quote(text),
        Json::List(_) | Json::Object(_) => quote(&serde_json::to_string(value).unwrap_or_default()),
    }
}
fn check_transport(fields: &Object, stdio: &StdioPolicy) -> Result<(), RequestError> {
    let Some(transport) = fields.get("transport").and_then(Json::as_str) else {
        return Ok(());
    };
    match transport {
        "http" | "sse" => check_remote_url(fields),
        "stdio" => check_stdio(fields, stdio),
        _ => Ok(()),
    }
}
fn check_remote_url(fields: &Object) -> Result<(), RequestError> {
    let url = fields.get("url").is_some_and(Json::is_truthy);
    let spec_path = fields.get("spec_path").is_some_and(Json::is_truthy);
    if url || spec_path {
        return Ok(());
    }
    Err(RequestError::Value(
        "url or spec_path is required for HTTP/SSE transport".to_owned(),
    ))
}
fn check_stdio(fields: &Object, stdio: &StdioPolicy) -> Result<(), RequestError> {
    if !stdio.enabled {
        return Err(RequestError::Value(MCP_STDIO_DISABLED_MESSAGE.to_owned()));
    }
    let Some(raw_command) = fields.get("command").filter(|value| value.is_truthy()) else {
        return Err(RequestError::Value(
            "command is required for stdio transport".to_owned(),
        ));
    };
    if !fields.get("args").is_some_and(Json::is_truthy) {
        return Err(RequestError::Value(
            "args is required for stdio transport".to_owned(),
        ));
    }
    let Some(command) = raw_command.as_str() else {
        return Ok(());
    };
    let basename = command.rsplit_once('/').map_or(command, |(_, tail)| tail);
    if stdio.allowed_commands.contains(basename) {
        return Ok(());
    }
    Err(RequestError::Value(format!(
        "Command '{command}' is not in the allowed commands list for stdio transport. Allowed commands: {}",
        quoted_list(stdio.allowed_commands.iter())
    )))
}
