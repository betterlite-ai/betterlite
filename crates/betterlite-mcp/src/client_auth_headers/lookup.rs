mod tests;

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use subtle::ConstantTimeEq as _;
use tracing::debug;

// alias, then header name, then value, as B4654's request_headers reader builds it
pub type ServerAuthHeaders = BTreeMap<String, BTreeMap<String, SecretString>>;

// LiteLLM's sanitize_mcp_alias_for_header, which the dashboard's mcpHeaderUtils.ts mirrors
#[must_use]
pub fn sanitize_alias_for_header(alias: &str) -> String {
    let mut sanitized = String::with_capacity(alias.len());
    for character in alias.to_lowercase().trim().chars() {
        if character.is_ascii_lowercase() || character.is_ascii_digit() {
            sanitized.push(character);
            continue;
        }
        if !sanitized.ends_with('_') {
            sanitized.push('_');
        }
    }
    sanitized.trim_matches('_').to_owned()
}

// LiteLLM's _header_keys_for_identifier: the raw identifier first, then its header-safe form
#[must_use]
pub fn header_keys(identifier: &str) -> Vec<String> {
    let lowered = identifier.to_lowercase();
    let sanitized = sanitize_alias_for_header(identifier);
    if sanitized.is_empty() || sanitized == lowered {
        vec![lowered]
    } else {
        vec![lowered, sanitized]
    }
}

// LiteLLM's lookup_mcp_server_auth_in_headers
#[must_use]
pub fn lookup_server_auth<'headers>(
    headers: &'headers ServerAuthHeaders,
    alias: Option<&str>,
    server_name: Option<&str>,
    access_groups: &[String],
) -> Option<&'headers BTreeMap<String, SecretString>> {
    if headers.is_empty() {
        return None;
    }
    // B4654's reader lowercases every alias, so two keys never fold into one here
    let normalized: BTreeMap<String, &BTreeMap<String, SecretString>> = headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect();
    for identifier in [alias, server_name] {
        let Some(candidate) = identifier.filter(|text| !text.is_empty()) else {
            continue;
        };
        if let Some(found) = matching_header(&normalized, candidate) {
            return Some(found);
        }
    }
    group_default(&normalized, alias, access_groups)
}

fn matching_header<'headers>(
    normalized: &BTreeMap<String, &'headers BTreeMap<String, SecretString>>,
    identifier: &str,
) -> Option<&'headers BTreeMap<String, SecretString>> {
    header_keys(identifier)
        .into_iter()
        .find_map(|key| normalized.get(&key).copied())
}

// LiteLLM's group default: it is used only when every matching group carries the same map
fn group_default<'headers>(
    normalized: &BTreeMap<String, &'headers BTreeMap<String, SecretString>>,
    alias: Option<&str>,
    access_groups: &[String],
) -> Option<&'headers BTreeMap<String, SecretString>> {
    let mut group_matches: Vec<&BTreeMap<String, SecretString>> = Vec::new();
    for group in access_groups {
        if let Some(found) = matching_header(normalized, group) {
            group_matches.push(found);
        }
    }
    let first = group_matches.first().copied()?;
    if group_matches
        .iter()
        .skip(1)
        .all(|other| header_maps_equal(first, other))
    {
        return Some(first);
    }
    debug!(
        "Ambiguous MCP group auth headers for server alias={} (groups={access_groups:?}); not forwarding any group credential",
        alias.unwrap_or_default()
    );
    None
}

// secrecy's SecretString has no PartialEq, so the whole map compares through the exposed values
fn header_maps_equal(
    left: &BTreeMap<String, SecretString>,
    right: &BTreeMap<String, SecretString>,
) -> bool {
    left.len() == right.len()
        && left.iter().all(|(name, value)| {
            right.get(name).is_some_and(|other| {
                bool::from(
                    value
                        .expose_secret()
                        .as_bytes()
                        .ct_eq(other.expose_secret().as_bytes()),
                )
            })
        })
}
