use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::ui_session_identity::is_ui_session_credential;
use betterlite_config::sections::settings::{JsonValue, SettingsStore};
use betterlite_db::identity_rows::Payload;
use betterlite_mcp::request_context::allowlist::{
    CLIENT_ID_JWT_FIELD_SETTING, JwtAuthSettings, MCP_ALLOWED_CLIENTS_SETTING,
    MCP_CLIENT_ID_HEADER_SETTING, McpAllowedClient, McpClientAllowlist, McpClientAllowlistSettings,
    check_mcp_client_allowed, load_mcp_client_allowlist,
};
use http::HeaderMap;
use http::header::ORIGIN;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::{McpHttpError, latin1_header_text};

mod tests;

const INVALID_ORIGIN_DETAIL: &str = "Invalid Origin header";
const INVALID_REQUEST: i64 = -32600;
const JWT_AUTH_SETTING: &str = "litellm_jwtauth";
const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const SERVER_ERROR_STATUS: u16 = 500;

// LiteLLM's reject_disallowed_mcp_origin reads every Origin header, not just the first
pub fn reject_disallowed_origin(
    origins: &[String],
    headers: &HeaderMap,
) -> Result<(), McpHttpError> {
    if origins.iter().any(|allowed| allowed == "*") {
        return Ok(());
    }
    let refused = headers
        .get_all(ORIGIN)
        .iter()
        .map(latin1_header_text)
        .any(|origin| !origins.iter().any(|allowed| *allowed == origin));
    if refused {
        return Err(McpHttpError {
            status: 403,
            detail: JsonValue::String(INVALID_ORIGIN_DETAIL.to_owned()),
            headers: Vec::new(),
        });
    }
    Ok(())
}

pub fn unsupported_protocol_version(headers: &HeaderMap, configured: &[String]) -> Option<String> {
    let values = headers.get_all(PROTOCOL_VERSION_HEADER);
    for value in values.iter() {
        let decoded = latin1_header_text(value);
        let trimmed = decoded.trim();
        if !trimmed.is_empty() && !configured.iter().any(|supported| supported == trimmed) {
            return Some(trimmed.to_owned());
        }
    }
    None
}

#[derive(Serialize)]
struct VersionRefusalBody {
    jsonrpc: &'static str,
    id: (),
    error: VersionRefusalError,
}

#[derive(Serialize)]
struct VersionRefusalError {
    code: i64,
    message: String,
}

pub fn version_refusal(value: &str, configured: &[String]) -> McpHttpError {
    let message = format!(
        "Unsupported MCP-Protocol-Version {value}; supported: {}",
        configured.join(", ")
    );
    let body = VersionRefusalBody {
        jsonrpc: "2.0",
        id: (),
        error: VersionRefusalError {
            code: INVALID_REQUEST,
            message,
        },
    };
    McpHttpError {
        status: 400,
        detail: serde_json::to_value(body).unwrap_or(JsonValue::Null),
        headers: Vec::new(),
    }
}

pub fn proxy_error_to_http(code: Option<&str>, message: &str, headers: &HeaderMap) -> McpHttpError {
    let status = code
        .and_then(|code| code.trim().parse::<u16>().ok())
        .filter(|status| (100..=599).contains(status))
        .unwrap_or(SERVER_ERROR_STATUS);
    McpHttpError {
        status,
        detail: JsonValue::String(message.to_owned()),
        headers: headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), latin1_header_text(value)))
            .collect(),
    }
}

#[derive(Default, Deserialize)]
struct ClientAuthView {
    #[serde(default)]
    jwt_claims: Option<Payload>,
}

fn optional_setting_name(setting: &str, value: Option<&JsonValue>) -> Option<String> {
    let Some(value) = value else {
        return None;
    };
    match value {
        JsonValue::Null => None,
        JsonValue::String(name) => Some(name.clone()),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            warn!("{setting} is not a string; ignoring it");
            None
        }
    }
}

// _load_mcp_client_allowlist re-reads general_settings on every request, so the caller passes its current snapshot
pub fn allowlist_from_settings(general_settings: &SettingsStore) -> Option<McpClientAllowlist> {
    let Some(raw_clients) = general_settings.get(MCP_ALLOWED_CLIENTS_SETTING) else {
        return None;
    };
    if raw_clients == &JsonValue::Null {
        return None;
    }
    let mcp_allowed_clients = match serde_json::from_value::<Vec<McpAllowedClient>>(
        raw_clients.clone(),
    ) {
        Ok(clients) => Some(clients),
        Err(_) => {
            warn!(
                "{MCP_ALLOWED_CLIENTS_SETTING} is not a list of {{alias, value}} entries; rejecting every MCP client until it is fixed"
            );
            Some(Vec::new())
        }
    };
    let mut jwt_auth = None;
    if let Some(JsonValue::Object(fields)) = general_settings.get(JWT_AUTH_SETTING) {
        let field = optional_setting_name(
            &format!("{JWT_AUTH_SETTING}.{CLIENT_ID_JWT_FIELD_SETTING}"),
            fields.get(CLIENT_ID_JWT_FIELD_SETTING),
        );
        jwt_auth = Some(JwtAuthSettings {
            mcp_client_id_jwt_field: field,
        });
    }
    let settings = McpClientAllowlistSettings {
        mcp_allowed_clients,
        mcp_client_id_header: optional_setting_name(
            MCP_CLIENT_ID_HEADER_SETTING,
            general_settings.get(MCP_CLIENT_ID_HEADER_SETTING),
        ),
        litellm_jwtauth: jwt_auth,
    };
    load_mcp_client_allowlist(&settings)
}

// reject_disallowed_mcp_client: the dashboard's own session is not a client application
pub fn reject_disallowed_client(
    allowlist: Option<&McpClientAllowlist>,
    headers: &HeaderMap,
    auth: Option<&UserAPIKeyAuth>,
) -> Result<(), McpHttpError> {
    if auth.is_some_and(is_ui_session_credential) {
        return Ok(());
    }
    let claims: Option<Payload> = auth
        .and_then(|auth| serde_json::to_value(auth).ok())
        .and_then(|value| serde_json::from_value::<ClientAuthView>(value).ok())
        .and_then(|view| view.jwt_claims);
    let Err(rejection) = check_mcp_client_allowed(allowlist, claims.as_ref(), headers) else {
        return Ok(());
    };
    warn!(
        "Rejected MCP request from a disallowed client application: {}",
        rejection.details
    );
    Err(McpHttpError {
        status: 403,
        detail: serde_json::to_value(rejection.response_body()).unwrap_or(JsonValue::Null),
        headers: Vec::new(),
    })
}
