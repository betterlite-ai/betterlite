use std::collections::BTreeMap;
use std::str::from_utf8;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use http::HeaderMap;
use http::header::AUTHORIZATION;
use regex::Regex;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::latin1_header_text;

mod tests;

const LITELLM_KEY_HEADER: &str = "x-litellm-api-key";
const SESSION_ID_HEADER: &str = "mcp-session-id";

// Python anchors $ before a trailing newline and Rust does not, but a request-target never carries one
const MCP_PATH_PATTERN: &str = r"^/mcp/([^?#]+)(?:\?.*)?(?:#.*)?$";
const COMMA_TAIL_PATTERN: &str = "/([^/,]+(?:/[^/,]+)*)$";
const SINGLE_NAME_PATTERN: &str = "^([^/]+(?:/[^/]+)?)(?:/.*)?$";

// _get_mcp_servers_in_path: None means "use the x-mcp-servers header", Some(empty) overrides it with nothing
pub fn servers_in_path(path: &str) -> Option<Vec<String>> {
    let trimmed = path.trim_end_matches('/');
    if trimmed == "/mcp/sse" || trimmed == "/mcp/sse/messages" {
        return None;
    }
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    if let (Some(first), Some(second)) = (segments.next(), segments.next())
        && second == "mcp"
        && first != "mcp"
    {
        return Some(vec![first.to_owned()]);
    }
    let mcp_path = Regex::new(MCP_PATH_PATTERN).ok()?;
    let captured = mcp_path.captures(path)?.get(1)?.as_str();
    if captured.contains(',') {
        let tail = Regex::new(COMMA_TAIL_PATTERN).ok()?;
        let servers_part = match tail.find(captured) {
            Some(found) => captured.get(..found.start())?,
            None => captured,
        };
        return Some(
            servers_part
                .split(',')
                .map(str::trim)
                .filter(|server| !server.is_empty())
                .map(str::to_owned)
                .collect(),
        );
    }
    let single_name = Regex::new(SINGLE_NAME_PATTERN).ok()?;
    let found_name = single_name
        .captures(captured)
        .and_then(|found| found.get(1));
    found_name.map_or_else(
        || Some(vec![captured.to_owned()]),
        |name| Some(vec![name.as_str().to_owned()]),
    )
}

pub fn session_id_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SESSION_ID_HEADER)
        .iter()
        .next()
        .and_then(|value| from_utf8(value.as_bytes()).ok())
        .map(str::to_owned)
}

#[derive(Default, Deserialize)]
struct OwnerFields {
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

// _owner_fingerprint_for: first present credential wins, and an empty string is no credential
#[must_use]
pub fn owner_fingerprint(
    auth: Option<&UserAPIKeyAuth>,
    oauth2_headers: Option<&BTreeMap<String, String>>,
    client_ip: Option<&str>,
) -> String {
    let fields: OwnerFields = auth
        .and_then(|credential| serde_json::to_value(credential).ok())
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    if let Some(api_key) = fields.api_key.as_deref().filter(|key| !key.is_empty()) {
        let hash = sha256_hex(api_key);
        return format!("key:{hash}");
    }
    if let Some(user_id) = fields.user_id.as_deref().filter(|id| !id.is_empty()) {
        let hash = sha256_hex(user_id);
        return format!("user:{hash}");
    }
    if let Some(headers) = oauth2_headers {
        let upper = headers
            .get("Authorization")
            .filter(|value| !value.is_empty());
        let presented = upper
            .or_else(|| headers.get("authorization"))
            .filter(|value| !value.is_empty());
        if let Some(authorization) = presented {
            let hash = sha256_hex(authorization);
            return format!("oauth:{hash}");
        }
    }
    if let Some(ip) = client_ip.filter(|ip| !ip.is_empty()) {
        let hash = sha256_hex(ip);
        return format!("ip:{hash}");
    }
    "anonymous".to_owned()
}

pub fn authorization_header(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(AUTHORIZATION)
        .iter()
        .next()
        .map(latin1_header_text)
}

// _get_forwarded_auth_from_scope: Authorization is the upstream token only next to x-litellm-api-key
#[must_use]
pub fn forwarded_authorization(headers: &HeaderMap) -> Option<String> {
    if !headers.contains_key(LITELLM_KEY_HEADER) {
        return None;
    }
    authorization_header(headers)
}
