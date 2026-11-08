use betterlite_db::mcp_stored_contracts::MCPAuth;

pub const UPSTREAM_DISCOVERY_AUTH_TYPES: [MCPAuth; 3] = [
    MCPAuth::Oauth2,
    MCPAuth::TruePassthrough,
    MCPAuth::OauthDelegate,
];

#[must_use]
pub fn blank_to_none(value: Option<&str>) -> Option<&str> {
    let text = value?.trim();
    if text.is_empty() {
        return None;
    }
    Some(text)
}

#[must_use]
pub fn uses_issuer_anchor(issuer: Option<&str>, is_discovery_auth_type: bool) -> bool {
    if blank_to_none(issuer).is_none() {
        return false;
    }
    is_discovery_auth_type
}

#[must_use]
pub fn has_discovery_source(server_url: Option<&str>, use_issuer_anchor: bool) -> bool {
    if server_url.is_some_and(|url| !url.is_empty()) {
        return true;
    }
    use_issuer_anchor
}

#[must_use]
pub fn obo_needs_discovery(
    auth_type: Option<MCPAuth>,
    token_exchange_endpoint: Option<&str>,
    token_url: Option<&str>,
) -> bool {
    if auth_type != Some(MCPAuth::Oauth2TokenExchange) {
        return false;
    }
    blank_to_none(token_exchange_endpoint).is_none() && blank_to_none(token_url).is_none()
}
