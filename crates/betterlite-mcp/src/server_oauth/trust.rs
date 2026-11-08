use super::basics::blank_to_none;
use super::issuer::normalized_authorize_endpoint;
use crate::server_record::{MCPOAuthMetadata, MCPServer};

#[must_use]
pub fn restrict_to_corroborated(
    metadata: Option<MCPOAuthMetadata>,
    manual_authorization_url: Option<&str>,
    server_identifier: &str,
    is_dcr_bridge: bool,
) -> Option<MCPOAuthMetadata> {
    let document = metadata?;
    let Some(manual) = blank_to_none(manual_authorization_url) else {
        return Some(document);
    };
    if endpoints_corroborate(document.authorization_url.as_deref(), Some(manual)) {
        return Some(document);
    }
    let discovered_token = blank_to_none(document.token_url.as_deref());
    let discovered_registration = blank_to_none(document.registration_url.as_deref());
    if discovered_token.is_none() && discovered_registration.is_none() {
        return Some(document);
    }
    let advertised = blank_to_none(document.authorization_url.as_deref())
        .map_or_else(|| "<absent>".to_owned(), normalized_authorize_endpoint);
    let pinned = normalized_authorize_endpoint(manual);
    let bridge_note = if is_dcr_bridge && discovered_registration.is_some() {
        " The discovered registration_url is rejected with it, so this dcr_bridge server stays on the short-circuit registration arm."
    } else {
        ""
    };
    tracing::warn!(
        "MCP OAuth discovery for server {server_identifier} advertised authorization_endpoint {advertised}, which does not match the manually configured authorization_url {pinned}; rejecting the discovered token_url/registration_url so authorization codes and client credentials only follow the configured authorization server. Configure Token URL manually if the mismatch is intentional.{bridge_note}"
    );
    let mut restricted = document;
    restricted.token_url = None;
    restricted.registration_url = None;
    Some(restricted)
}

#[must_use]
pub fn endpoints_corroborate(
    source_authorization_url: Option<&str>,
    trusted_authorization_url: Option<&str>,
) -> bool {
    let Some(trusted) = blank_to_none(trusted_authorization_url) else {
        return true;
    };
    let Some(source) = blank_to_none(source_authorization_url) else {
        return false;
    };
    normalized_authorize_endpoint(source) == normalized_authorize_endpoint(trusted)
}

pub fn carry_forward_endpoints(server: &mut MCPServer, previous: Option<&MCPServer>) {
    let Some(last) = previous else {
        return;
    };
    if last.fields.url != server.fields.url || last.fields.auth_type != server.fields.auth_type {
        return;
    }
    if server.fields.issuer_is_anchored {
        if server.fields.scopes.as_ref().is_none_or(Vec::is_empty)
            && last
                .fields
                .scopes
                .as_ref()
                .is_some_and(|scopes| !scopes.is_empty())
        {
            server.fields.scopes.clone_from(&last.fields.scopes);
        }
        return;
    }
    let may_carry = endpoints_corroborate(
        last.fields.authorization_url.as_deref(),
        server.fields.authorization_url.as_deref(),
    );
    if server.fields.authorization_url.is_none()
        && last
            .fields
            .authorization_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
    {
        server
            .fields
            .authorization_url
            .clone_from(&last.fields.authorization_url);
    }
    carry_missing(server, last, may_carry);
}

fn carry_missing(server: &mut MCPServer, last: &MCPServer, may_carry: bool) {
    if !may_carry {
        return;
    }
    copy_missing(
        &mut server.fields.token_url,
        last.fields.token_url.as_deref(),
    );
    copy_missing(
        &mut server.fields.registration_url,
        last.fields.registration_url.as_deref(),
    );
    if server.fields.scopes.as_ref().is_none_or(Vec::is_empty)
        && last
            .fields
            .scopes
            .as_ref()
            .is_some_and(|scopes| !scopes.is_empty())
    {
        server.fields.scopes.clone_from(&last.fields.scopes);
    }
}

fn copy_missing(target: &mut Option<String>, previous: Option<&str>) {
    if target.is_none() && previous.is_some_and(|value| !value.is_empty()) {
        *target = previous.map(str::to_owned);
    }
}
