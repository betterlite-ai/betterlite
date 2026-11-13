use super::basics::blank_to_none;
use crate::oauth_resource::resource_identity::redact_mcp_resource_url;
use crate::server_record::{MCPOAuthMetadata, ServerFlag};

#[derive(Clone, Copy)]
pub struct UnresolvedCheck<'check> {
    pub server_ref: &'check str,
    pub server_url: Option<&'check str>,
    pub discovery_attempted: ServerFlag,
    pub discovery_deferred: ServerFlag,
    pub issuer_anchored: ServerFlag,
    pub metadata: Option<&'check MCPOAuthMetadata>,
    pub needs_authorization_url: ServerFlag,
    pub needs_token_url: ServerFlag,
    pub manual_authorization_url: Option<&'check str>,
    pub manual_token_url: Option<&'check str>,
}

#[derive(Clone, Copy)]
pub struct NeededEndpoint<'endpoint> {
    pub needed: bool,
    pub manual: Option<&'endpoint str>,
}

#[must_use]
pub fn discovery_leaves_unresolved(
    authorization: NeededEndpoint<'_>,
    token: NeededEndpoint<'_>,
) -> bool {
    (authorization.needed && blank_to_none(authorization.manual).is_none())
        || (token.needed && blank_to_none(token.manual).is_none())
}

pub fn warn_unresolved(check: UnresolvedCheck<'_>) {
    if check.discovery_deferred || check.issuer_anchored {
        return;
    }
    let manual_authorization = blank_to_none(check.manual_authorization_url);
    let manual_token = blank_to_none(check.manual_token_url);
    let discovered_authorization = check
        .metadata
        .and_then(|metadata| blank_to_none(metadata.authorization_url.as_deref()));
    let discovered_token = check
        .metadata
        .and_then(|metadata| blank_to_none(metadata.token_url.as_deref()));
    let mut unresolved = Vec::new();
    if check.needs_authorization_url
        && manual_authorization.is_none()
        && discovered_authorization.is_none()
    {
        unresolved.push("authorization_url");
    }
    if check.needs_token_url && manual_token.is_none() && discovered_token.is_none() {
        unresolved.push("token_url");
    }
    if unresolved.is_empty() {
        return;
    }
    let server_ref = check.server_ref;
    if check.discovery_attempted {
        let fields = unresolved.join(", ");
        let origin =
            redact_mcp_resource_url(check.server_url).unwrap_or_else(|| "<no url>".to_owned());
        tracing::warn!(
            "MCP server {server_ref}: OAuth endpoint discovery left {fields} unresolved (server url origin: {origin}). OAuth flows that need them will fail with 'not configured' errors until they resolve. Check the preceding 'MCP OAuth' log lines for why discovery failed, verify the configured server url, or set the unresolved endpoint urls manually, or set issuer to discover them from the identity provider (RFC 8414)"
        );
        return;
    }
    let fields = unresolved.join(" and ");
    let verb = if unresolved.len() == 1 { "is" } else { "are" };
    tracing::warn!(
        "MCP server {server_ref} uses OAuth but has no discovery source (no server url or pinned issuer), and {fields} {verb} not set manually. Set the missing endpoint urls on the server, or set issuer to discover them from the identity provider (RFC 8414)"
    );
}
