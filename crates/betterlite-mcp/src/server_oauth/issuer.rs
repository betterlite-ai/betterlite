use betterlite_db::mcp_stored_contracts::JsonValue;

use super::basics::blank_to_none;
use crate::oauth_resource::resource_identity::canonicalize_url_identity;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ManualEndpoints<'endpoint> {
    pub authorization_url: Option<&'endpoint str>,
    pub token_url: Option<&'endpoint str>,
    pub registration_url: Option<&'endpoint str>,
}

#[must_use]
pub fn endpoints_yield_to_issuer<'endpoint>(
    issuer: Option<&str>,
    is_discovery_auth_type: bool,
    endpoints: ManualEndpoints<'endpoint>,
    server_ref: &str,
) -> ManualEndpoints<'endpoint> {
    if blank_to_none(issuer).is_none() || !is_discovery_auth_type {
        return endpoints;
    }
    let mut discarded = Vec::new();
    if endpoints
        .authorization_url
        .is_some_and(|url| !url.is_empty())
    {
        discarded.push("authorization_url");
    }
    if endpoints.token_url.is_some_and(|url| !url.is_empty()) {
        discarded.push("token_url");
    }
    if endpoints
        .registration_url
        .is_some_and(|url| !url.is_empty())
    {
        discarded.push("registration_url");
    }
    if !discarded.is_empty() {
        discarded.sort_unstable();
        let fields = discarded.join(", ");
        let verb = if discarded.len() == 1 { "is" } else { "are" };
        tracing::warn!(
            "MCP server {server_ref} has a pinned Issuer, so its stored {fields} {verb} not used: an anchored issuer is the sole endpoint source (RFC 8414 section 3.3) and a failed issuer fetch fails closed rather than falling back to them. To use manually configured endpoints instead, clear the Issuer field and re-enter the endpoint urls (clearing the Issuer also clears endpoints that may have been resolved under it), or clear the Issuer alone to re-discover from the server url"
        );
    }
    ManualEndpoints {
        authorization_url: None,
        token_url: None,
        registration_url: None,
    }
}

#[must_use]
pub fn normalized_authorize_endpoint(url: &str) -> String {
    // comparisons only: an unparsable url keeps its text, and the warning prints a placeholder
    let Ok(identity) = canonicalize_url_identity(url) else {
        return url.to_owned();
    };
    identity
}

#[must_use]
pub fn issuer_matches(claimed_issuer: Option<&JsonValue>, configured_issuer: &str) -> bool {
    let Some(JsonValue::String(claimed)) = claimed_issuer else {
        return false;
    };
    if claimed.is_empty() {
        return false;
    }
    normalized_authorize_endpoint(claimed) == normalized_authorize_endpoint(configured_issuer)
}
