use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::mcp_stored_contracts::JsonValue;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use regex::{Captures, Regex};
use thiserror::Error;

mod tests;

const ENV_VAR_PATTERN: &str = r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}";
// Python's quote(safe="") leaves letters, digits and _.-~ alone and percent-encodes every other UTF-8 byte
const SETUP_URL_BYTES: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserEnvSpec {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Error)]
#[error(
    "{}",
    missing_user_env_vars_message(
        .server_id.as_str(),
        .server_name.as_deref(),
        .missing.as_slice(),
        .setup_url.as_str()
    )
)]
pub struct MCPMissingUserEnvVarsError {
    pub server_id: String,
    pub server_name: Option<String>,
    pub missing: Vec<String>,
    pub setup_url: String,
}

#[must_use]
pub fn parse_admin_env_vars(
    env_vars: Option<&[JsonValue]>,
) -> (BTreeMap<String, String>, Vec<UserEnvSpec>) {
    let mut globals = BTreeMap::new();
    let mut user_specs = Vec::new();
    for entry in env_vars.unwrap_or_default() {
        let JsonValue::Object(fields) = entry else {
            continue;
        };
        let Some(JsonValue::String(name)) = fields.get("name") else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let user_scoped = matches!(
            fields.get("scope"),
            Some(JsonValue::String(scope)) if scope.as_str() == "user"
        );
        if user_scoped {
            user_specs.push(UserEnvSpec {
                name: name.clone(),
                description: match fields.get("description") {
                    Some(JsonValue::String(text)) => Some(text.clone()),
                    _ => None,
                },
            });
        } else {
            globals.insert(name.clone(), stored_text(fields.get("value")));
        }
    }
    (globals, user_specs)
}

#[must_use]
pub fn find_env_var_references(value: &str) -> BTreeSet<String> {
    if value.is_empty() {
        return BTreeSet::new();
    }
    let Ok(pattern) = Regex::new(ENV_VAR_PATTERN) else {
        return BTreeSet::new();
    };
    pattern
        .captures_iter(value)
        .filter_map(|captures| captures.get(1))
        .map(|name| name.as_str().to_owned())
        .collect()
}

#[must_use]
pub fn collect_env_var_references<'text, I>(strings: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = &'text str>,
{
    let mut references = BTreeSet::new();
    for text in strings {
        references.extend(find_env_var_references(text));
    }
    references
}

#[must_use]
pub fn interpolate_env_vars(value: &str, variables: &BTreeMap<String, String>) -> String {
    if value.is_empty() {
        return value.to_owned();
    }
    let Ok(pattern) = Regex::new(ENV_VAR_PATTERN) else {
        return value.to_owned();
    };
    pattern
        .replace_all(value, |captures: &Captures<'_>| {
            let Some(whole) = captures.get(0) else {
                return String::new();
            };
            let Some(name) = captures.get(1) else {
                return whole.as_str().to_owned();
            };
            variables
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(|| whole.as_str().to_owned())
        })
        .into_owned()
}

#[must_use]
pub fn interpolate_headers(
    headers: &BTreeMap<String, String>,
    variables: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), interpolate_env_vars(value, variables)))
        .collect()
}

#[must_use]
pub fn build_env_var_setup_url(proxy_base_url: Option<&str>, server_id: &str) -> String {
    let base = proxy_base_url.unwrap_or_default().trim_end_matches('/');
    let path = format!(
        "/ui/mcp-servers?fill_env_vars={}",
        utf8_percent_encode(server_id, SETUP_URL_BYTES)
    );
    if base.is_empty() {
        path
    } else {
        format!("{base}{path}")
    }
}

fn missing_user_env_vars_message(
    server_id: &str,
    server_name: Option<&str>,
    missing: &[String],
    setup_url: &str,
) -> String {
    let label = server_name
        .filter(|name| !name.is_empty())
        .unwrap_or(server_id);
    let bullets = missing
        .iter()
        .map(|name| format!("- {name}"))
        .collect::<Vec<String>>()
        .join("\n");
    format!(
        "Cannot connect to MCP server \"{label}\".\n\nYour administrator configured this server to require per-user variables, but you haven't set the following yet:\n{bullets}\n\nSet your credentials here:\n{setup_url}"
    )
}

fn stored_text(value: Option<&JsonValue>) -> String {
    match value {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}
