use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::server_record::MCPServer;
use crate::wire_compat::errors::HttpException;

const UNAUTHORIZED_DETAIL: &str = "Unauthorized";

// A scalar SERVER_ROOT_PATH registers the well-known routes with the prefix inserted (RFC 8414), while a per-request prefix is stripped before routing, so the two deployments need different URLs for the same document.
#[must_use]
pub fn oauth_protected_resource_path(
    root_path: &str,
    scalar_root_path: &str,
    server: &MCPServer,
) -> String {
    let prefix = if root_path == "/" { "" } else { root_path };
    let name = resource_name(server);
    let scalar = scalar_root_path.trim_end_matches('/');
    let inserted = prefix.is_empty() || prefix == scalar;
    if inserted {
        return format!("/.well-known/oauth-protected-resource{prefix}/mcp/{name}");
    }
    format!("{prefix}/.well-known/oauth-protected-resource/mcp/{name}")
}

#[must_use]
pub fn raise_user_oauth_challenge(
    server: &MCPServer,
    root_path: &str,
    scalar_root_path: &str,
) -> HttpException {
    let resource_metadata = oauth_protected_resource_path(root_path, scalar_root_path, server);
    let challenge = format!("Bearer resource_metadata=\"{resource_metadata}\"");
    challenge_exception(challenge)
}

#[must_use]
pub fn raise_token_exchange_challenge(
    server: &MCPServer,
    root_path: &str,
    scalar_root_path: &str,
    claims: Option<&str>,
) -> HttpException {
    let resource_metadata = oauth_protected_resource_path(root_path, scalar_root_path, server);
    let encoded_claims = claims
        .filter(|blob| !blob.is_empty())
        .map(|blob| STANDARD.encode(blob.as_bytes()));
    let error = if encoded_claims.is_some() {
        "insufficient_claims"
    } else {
        "invalid_token"
    };
    let description = if encoded_claims.is_some() {
        "Step-up authentication required; satisfy the returned claims challenge with the IdP and retry"
    } else {
        "Missing or invalid subject token; authenticate with the IdP and retry"
    };
    let mut challenge = format!(
        "Bearer resource_metadata=\"{resource_metadata}\", error=\"{error}\", error_description=\"{description}\""
    );
    if let Some(encoded) = &encoded_claims {
        challenge.push_str(", claims=\"");
        challenge.push_str(encoded);
        challenge.push('"');
    }
    challenge_exception(challenge)
}

fn resource_name(server: &MCPServer) -> String {
    let candidates = [
        server.fields.alias.as_deref(),
        server.fields.server_name.as_deref(),
        Some(server.fields.name.as_str()),
        Some(server.fields.server_id.as_str()),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn challenge_exception(challenge: String) -> HttpException {
    let headers = BTreeMap::from([("www-authenticate".to_owned(), challenge)]);
    HttpException {
        status_code: 401,
        detail: UNAUTHORIZED_DETAIL.to_owned(),
        headers,
    }
}
