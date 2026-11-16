use betterlite_prompts::arguments::json::{Json, Object};

use crate::server_payload::RequestError;
use crate::server_payload::repr;
use crate::server_payload::rules::{MCP_STDIO_DISABLED_MESSAGE, StdioPolicy};

const DISCOVERY_MESSAGE: &str = "per_server_oauth_discovery is only supported for auth_type oauth2 with oauth2_flow authorization_code and without delegate_auth_to_upstream.";

pub(super) fn check_new(value: &Json, stdio: &StdioPolicy) -> Result<(), RequestError> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    reject_discovery(fields, true)?;
    check_dcr_bridge(fields, true)?;
    check_transport(fields, stdio)
}

pub(super) fn check_update(value: &Json, stdio: &StdioPolicy) -> Result<(), RequestError> {
    let Some(fields) = value.as_object() else {
        return Ok(());
    };
    reject_discovery(fields, false)?;
    check_dcr_bridge(fields, false)?;
    check_transport(fields, stdio)
}

fn truthy(fields: &Object, key: &str) -> bool {
    let Some(value) = fields.get(key) else {
        return false;
    };
    value.is_truthy()
}

fn reject_discovery(fields: &Object, require_auth_type: bool) -> Result<(), RequestError> {
    if !truthy(fields, "per_server_oauth_discovery") {
        return Ok(());
    }
    let auth_type_ok = fields.get("auth_type").and_then(Json::as_str) == Some("oauth2")
        || (!require_auth_type && !fields.contains_key("auth_type"));
    let flow_ok = fields.get("oauth2_flow").and_then(Json::as_str) == Some("authorization_code")
        || (!require_auth_type && !fields.contains_key("oauth2_flow"));
    if auth_type_ok && flow_ok && !truthy(fields, "delegate_auth_to_upstream") {
        Ok(())
    } else {
        Err(RequestError::Value(DISCOVERY_MESSAGE.to_owned()))
    }
}

fn check_dcr_bridge(fields: &Object, require_auth_type: bool) -> Result<(), RequestError> {
    if !truthy(fields, "dcr_bridge") {
        return Ok(());
    }
    if !require_auth_type && !fields.contains_key("auth_type") {
        return Ok(());
    }
    let auth_type = fields.get("auth_type").cloned().unwrap_or(Json::Null);
    if matches!(
        auth_type.as_str(),
        Some("true_passthrough" | "oauth_delegate")
    ) {
        Ok(())
    } else {
        Err(RequestError::Value(dcr_bridge_message(&auth_type)))
    }
}

fn dcr_bridge_message(auth_type: &Json) -> String {
    format!(
        "dcr_bridge is only supported for auth_type true_passthrough or oauth_delegate (got {}). The DCR bridge serves gateway-hosted OAuth discovery for the client-forwarded token modes; interactive oauth2 servers already run the gateway authorization-code flow.",
        repr::value(auth_type)
    )
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
    if truthy(fields, "url") || truthy(fields, "spec_path") {
        Ok(())
    } else {
        Err(RequestError::Value(
            "url or spec_path is required for HTTP/SSE transport".to_owned(),
        ))
    }
}

fn check_stdio(fields: &Object, stdio: &StdioPolicy) -> Result<(), RequestError> {
    if !stdio.enabled {
        return Err(RequestError::Value(MCP_STDIO_DISABLED_MESSAGE.to_owned()));
    }
    let Some(command) = fields.get("command").filter(|value| value.is_truthy()) else {
        return Err(RequestError::Value(
            "command is required for stdio transport".to_owned(),
        ));
    };
    if !truthy(fields, "args") {
        return Err(RequestError::Value(
            "args is required for stdio transport".to_owned(),
        ));
    }
    let Some(command) = command.as_str() else {
        return Ok(());
    };
    let basename = command.rsplit_once('/').map_or(command, |(_, tail)| tail);
    if stdio.allowed_commands.contains(basename) {
        Ok(())
    } else {
        Err(RequestError::Value(not_allowed_message(command, stdio)))
    }
}

fn not_allowed_message(command: &str, stdio: &StdioPolicy) -> String {
    let allowed = stdio
        .allowed_commands
        .iter()
        .cloned()
        .collect::<Vec<String>>();
    format!(
        "Command '{command}' is not in the allowed commands list for stdio transport. Allowed commands: {}",
        repr::list(&allowed)
    )
}
