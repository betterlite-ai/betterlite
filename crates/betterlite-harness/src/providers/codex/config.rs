#![expect(
    clippy::disallowed_types,
    reason = "Codex configuration overrides and the structured-output schema are caller-supplied JSON with no fixed shape"
)]
use serde_json::Value;

use crate::errors::HarnessError;
use crate::payloads::DEFAULT_MAX_RECURSE_DEPTH;
use crate::types::{Harness, JsonObject};

pub(super) const CODEX_TOKEN_ENV: &str = "LITELLM_HARNESS_TOKEN";
pub(super) const CODEX_SCHEMA_FILENAME: &str = "output_schema.json";
pub(super) const CODEX_HOME_ENV: &str = "CODEX_HOME";
const MANAGED_CONFIG_KEYS: [&str; 8] = [
    "model",
    "model_provider",
    "model_providers",
    "approval_policy",
    "sandbox_mode",
    "mcp_servers",
    "developer_instructions",
    "web_search",
];

#[must_use]
pub(super) fn toml_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || letter == '_' || letter == '-')
    {
        key.to_owned()
    } else {
        Value::String(key.to_owned()).to_string()
    }
}

pub(super) fn toml_value(raw: &Value, depth: usize) -> Result<String, HarnessError> {
    if depth > DEFAULT_MAX_RECURSE_DEPTH {
        return Err(HarnessError::OptionsMismatch {
            given: "CodexOptions.config value",
            harness: Harness::Codex,
            expected: "a value within the Codex config nesting limit",
        });
    }
    match raw {
        Value::Bool(flag) => Ok(flag.to_string()),
        Value::Number(number) => Ok(number.to_string()),
        Value::String(text) => Ok(Value::String(text.clone()).to_string()),
        Value::Array(items) => items
            .iter()
            .map(|item| toml_value(item, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(|encoded| format!("[{}]", encoded.join(", "))),
        Value::Object(entries) => entries
            .iter()
            .map(|(key, entry)| {
                Ok(format!(
                    "{} = {}",
                    toml_key(key),
                    toml_value(entry, depth.saturating_add(1))?
                ))
            })
            .collect::<Result<Vec<_>, HarnessError>>()
            .map(|encoded| format!("{{{}}}", encoded.join(", "))),
        Value::Null => Err(HarnessError::OptionsMismatch {
            given: "CodexOptions.config value",
            harness: Harness::Codex,
            expected: "a bool, number, string, array or object",
        }),
    }
}

#[must_use]
pub(super) fn toml_assignment(key: &str, value: &str) -> String {
    let quoted = Value::String(value.to_owned()).to_string();
    format!("{key}={quoted}")
}

#[must_use]
pub(super) fn schema_value(schema: &JsonObject) -> Value {
    let entries = schema
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()));
    Value::Object(entries.collect())
}

pub(super) fn config_overrides(config: &JsonObject) -> Result<Vec<String>, HarnessError> {
    let mut overrides = Vec::with_capacity(config.len());
    for (key, value) in config {
        let first = key.split('.').next().unwrap_or_default();
        if key.is_empty() || key.contains('=') {
            return Err(HarnessError::OptionsMismatch {
                given: "CodexOptions.config key",
                harness: Harness::Codex,
                expected: "a non-empty key without '='",
            });
        }
        if MANAGED_CONFIG_KEYS.contains(&first) {
            return Err(HarnessError::OptionsMismatch {
                given: "CodexOptions.config",
                harness: Harness::Codex,
                expected: "a key Codex does not manage itself",
            });
        }
        overrides.push(format!("{}={}", key, toml_value(value, 0)?));
    }
    Ok(overrides)
}

pub(super) fn pairs(flag: &str, values: &[String]) -> Vec<String> {
    let mut flagged = Vec::with_capacity(values.len().saturating_mul(2));
    for value in values {
        flagged.push(flag.to_owned());
        flagged.push(value.clone());
    }
    flagged
}
