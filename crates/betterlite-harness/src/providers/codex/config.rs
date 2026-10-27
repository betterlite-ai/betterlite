use serde_json::Value;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::options::{CodexOptions, ReasoningEffort};
use crate::sandbox::Sandbox;
use crate::types::{Harness, JsonObject, PermissionMode};

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

pub(super) fn mismatch() -> HarnessError {
    let error = HarnessError::OptionsMismatch {
        given: "CodexOptions.config",
        harness: Harness::Codex,
        expected: "valid Codex configuration",
    };
    error
}

pub fn toml_key(key: &str) -> String {
    if key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        key.to_owned()
    } else {
        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_owned())
    }
}

pub fn toml_value(value: &Value, depth: usize) -> Result<String, HarnessError> {
    if depth > crate::payloads::DEFAULT_MAX_RECURSE_DEPTH {
        return Err(mismatch());
    }
    match value {
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::String(value) => serde_json::to_string(value).map_err(|_| mismatch()),
        Value::Array(values) => values
            .iter()
            .map(|value| toml_value(value, depth.saturating_add(1)))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| format!("[{}]", values.join(", "))),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| {
                Ok(format!(
                    "{} = {}",
                    toml_key(key),
                    toml_value(value, depth.saturating_add(1))?
                ))
            })
            .collect::<Result<Vec<_>, HarnessError>>()
            .map(|values| format!("{{{}}}", values.join(", "))),
        Value::Null => Err(mismatch()),
    }
}

pub fn config_overrides(config: &JsonObject) -> Result<Vec<String>, HarnessError> {
    let overrides = config
        .iter()
        .map(|(key, value)| {
            let first = key.split('.').next().unwrap_or_default();
            if key.is_empty() || key.contains('=') || MANAGED_CONFIG_KEYS.contains(&first) {
                return Err(mismatch());
            }
            Ok(format!("{}={}", key, toml_value(value, 0)?))
        })
        .collect();
    overrides
}

pub(super) fn pairs(flag: &str, values: &[String]) -> Vec<String> {
    let pairs = values
        .iter()
        .flat_map(|value| [flag.to_owned(), value.clone()])
        .collect();
    pairs
}

pub(super) fn provider_overrides<S>(
    context: &SessionContext<S>,
) -> Result<Vec<String>, HarnessError> {
    let gateway = context.settings.gateway.as_ref().ok_or_else(mismatch)?;
    let base = format!("{}/v1", gateway.api_base.trim_end_matches('/'));
    Ok(vec![
        "model_provider=litellm".to_owned(),
        "model_providers.litellm.name=litellm".to_owned(),
        format!(
            "model_providers.litellm.base_url={}",
            toml_value(&Value::String(base), 0)?
        ),
        format!("model_providers.litellm.env_key={CODEX_TOKEN_ENV}"),
        "model_providers.litellm.wire_api=responses".to_owned(),
        "approval_policy=never".to_owned(),
    ])
}

pub(super) fn permission_args<S: Sandbox>(
    context: &SessionContext<S>,
    resumed: bool,
) -> Vec<String> {
    if context.settings.permissions != PermissionMode::ReadOnly && context.sandbox.is_container() {
        return vec!["--dangerously-bypass-approvals-and-sandbox".to_owned()];
    }
    let mode = match context.settings.permissions {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::Full | PermissionMode::Ask | PermissionMode::Edit => "workspace-write",
    };
    if resumed {
        vec!["-c".to_owned(), format!("sandbox_mode=\"{mode}\"")]
    } else {
        vec!["--sandbox".to_owned(), mode.to_owned()]
    }
}

pub(super) fn feature_overrides<S>(
    context: &SessionContext<S>,
    options: &CodexOptions,
) -> Vec<String> {
    let mut values = vec![format!(
        "web_search={}",
        if options.web_search {
            "live"
        } else {
            "disabled"
        }
    )];
    if let Some(effort) = options.reasoning_effort {
        let value = match effort {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
        };
        values.push(format!("model_reasoning_effort={value}"));
        values.push("model_reasoning_summary=auto".to_owned());
    }
    if let Some(instructions) = context.settings.instructions.as_ref() {
        if let Ok(encoded) = toml_value(&Value::String(instructions.clone()), 0) {
            values.push(format!("developer_instructions={encoded}"));
        }
    }
    values
}
