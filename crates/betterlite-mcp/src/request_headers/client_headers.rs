mod tests;

use std::collections::BTreeMap;

use tracing::{debug, warn};

use super::raw_headers::RawHeaders;

pub const LITELLM_API_KEY_HEADER: &str = "x-litellm-api-key";
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const MCP_AUTH_HEADER: &str = "x-mcp-auth";
pub const MCP_SERVERS_HEADER: &str = "x-mcp-servers";
pub const MCP_ACCESS_GROUPS_HEADER: &str = "x-mcp-access-groups";
const SERVER_AUTH_PREFIX: &str = "x-mcp-";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LitellmApiKey<'origin> {
    pub value: Option<&'origin str>,
    pub explicit_header_present: bool,
}

#[must_use]
pub fn get_litellm_api_key(headers: &RawHeaders) -> LitellmApiKey<'_> {
    let explicit = headers.lookup(LITELLM_API_KEY_HEADER);
    let value = explicit.filter(|value| !value.is_empty()).or_else(|| {
        headers
            .lookup(AUTHORIZATION_HEADER)
            .filter(|value| !value.is_empty())
    });
    LitellmApiKey {
        value,
        explicit_header_present: explicit.is_some(),
    }
}

#[must_use]
pub fn mcp_auth_header<'headers>(
    headers: &'headers RawHeaders,
    configured_name: &str,
) -> Option<&'headers str> {
    let found = headers.lookup(configured_name);
    if found.is_some_and(|text| !text.is_empty()) {
        warn!(
            header = configured_name,
            "the client-side MCP auth header is deprecated; use x-mcp-{{server_alias}}-{{header_name}} instead"
        );
    }
    found
}

#[must_use]
pub fn server_auth_headers(headers: &RawHeaders) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut servers: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (name, value) in headers.items() {
        let Some(remaining) = name.strip_prefix(SERVER_AUTH_PREFIX) else {
            continue;
        };
        if name == MCP_SERVERS_HEADER || name == MCP_ACCESS_GROUPS_HEADER {
            continue;
        }
        let Some((alias, raw_name)) = remaining.split_once('-') else {
            continue;
        };
        let header_name = if raw_name == "authorization" {
            "Authorization"
        } else {
            raw_name
        };
        servers
            .entry(alias.to_owned())
            .or_default()
            .insert(header_name.to_owned(), value.to_owned());
        debug!(alias, header = header_name, "found MCP server auth header");
    }
    servers
}

#[must_use]
pub fn oauth2_headers(headers: &RawHeaders) -> BTreeMap<String, String> {
    let mut oauth2 = BTreeMap::new();
    for (name, value) in headers.items() {
        if name.starts_with("authorization") {
            oauth2.insert("Authorization".to_owned(), value.to_owned());
        }
    }
    oauth2
}

#[must_use]
pub fn parse_mcp_servers_header(headers: &RawHeaders) -> Option<Vec<String>> {
    if let Some(header) = headers.lookup(MCP_SERVERS_HEADER) {
        return Some(split_list(header));
    }
    None
}

#[must_use]
pub fn parse_mcp_access_groups_header(headers: &RawHeaders) -> Option<Vec<String>> {
    if let Some(header) = headers.lookup(MCP_ACCESS_GROUPS_HEADER) {
        return Some(split_list(header));
    }
    None
}

pub(crate) fn split_list(header: &str) -> Vec<String> {
    header
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}
