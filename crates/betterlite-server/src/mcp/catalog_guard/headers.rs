use std::collections::{BTreeMap, BTreeSet};

const SPECIAL_HEADER_NAMES: &[&str] = &[
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
    "x-mcp-auth",
    "x-mcp-servers",
    "x-mcp-access-groups",
];
const CREDENTIAL_HEADER_NAMES: &[&str] = &[
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "x-litellm-api-key",
    "cookie",
    "proxy-authorization",
];
const MCP_SERVER_AUTH_HEADER_PREFIX: &str = "x-mcp-";
const MCP_SERVERS_HEADER: &str = "x-mcp-servers";
const MCP_ACCESS_GROUPS_HEADER: &str = "x-mcp-access-groups";
const DISABLE_REDACTION_HEADER: &str = "litellm-disable-message-redaction";
const REDACTED_VALUE: &str = "***REDACTED***";

pub struct HeaderSanitizeContext<'context> {
    pub extra_headers: &'context [String],
    pub identity_header_names: &'context [String],
    pub custom_key_header_name: Option<&'context str>,
    pub client_side_auth_header_name: &'context str,
}

#[must_use]
pub fn logging_safe_mcp_headers(
    raw_headers: &BTreeMap<String, String>,
    context: &HeaderSanitizeContext<'_>,
) -> BTreeMap<String, String> {
    let header_names: Vec<String> = raw_headers.keys().cloned().collect();
    let mut excluded = upstream_credential_headers(&header_names, context);
    excluded.insert(DISABLE_REDACTION_HEADER.to_owned());
    excluded.insert("host".to_owned());

    let cleaned = clean_headers(raw_headers, context);
    let filtered: BTreeMap<String, String> = cleaned
        .into_iter()
        .filter(|(name, _)| !excluded.contains(&name.to_lowercase()))
        .collect();
    redact_credential_headers(filtered)
}

fn forwarded_headers(context: &HeaderSanitizeContext<'_>) -> BTreeSet<String> {
    let mut exempt: BTreeSet<String> = context
        .identity_header_names
        .iter()
        .map(|name| name.to_lowercase())
        .collect();
    exempt.insert("authorization".to_owned());
    context
        .extra_headers
        .iter()
        .map(|name| name.to_lowercase())
        .filter(|name| !exempt.contains(name))
        .collect()
}

fn upstream_credential_headers(
    header_names: &[String],
    context: &HeaderSanitizeContext<'_>,
) -> BTreeSet<String> {
    let non_credential: BTreeSet<&str> =
        BTreeSet::from([MCP_SERVERS_HEADER, MCP_ACCESS_GROUPS_HEADER]);
    let client_side_auth = context.client_side_auth_header_name.to_lowercase();
    let forwarded_upstream = forwarded_headers(context);
    header_names
        .iter()
        .map(|name| name.to_lowercase())
        .filter(|name| {
            *name == client_side_auth
                || forwarded_upstream.contains(name)
                || (name.starts_with(MCP_SERVER_AUTH_HEADER_PREFIX)
                    && !non_credential.contains(name.as_str()))
        })
        .collect()
}

fn clean_headers(
    raw_headers: &BTreeMap<String, String>,
    context: &HeaderSanitizeContext<'_>,
) -> BTreeMap<String, String> {
    let custom_key_lower = context.custom_key_header_name.map(str::to_lowercase);
    let mut cleaned = BTreeMap::new();
    for (name, value) in raw_headers {
        let lower = name.to_lowercase();
        if SPECIAL_HEADER_NAMES.contains(&lower.as_str())
            || custom_key_lower.as_deref() == Some(lower.as_str())
        {
            continue;
        }
        cleaned.insert(lower, value.clone());
    }
    cleaned
}

fn redact_credential_headers(headers: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut redacted = BTreeMap::new();
    for (name, value) in headers {
        if CREDENTIAL_HEADER_NAMES.contains(&name.to_lowercase().as_str()) {
            redacted.insert(name, REDACTED_VALUE.to_owned());
        } else {
            redacted.insert(name, value);
        }
    }
    redacted
}
