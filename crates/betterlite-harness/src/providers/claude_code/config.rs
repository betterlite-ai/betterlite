use std::collections::BTreeMap;

use crate::errors::HarnessError;
use crate::options::ClaudeCodeOptions;
use crate::payloads::native_tool_names as shared_native_tool_names;
use crate::payloads::python_json::python_json_text;
use crate::types::{JsonObject, PermissionMode};

pub(super) const CLAUDE_BINARY: &str = "claude";
pub(super) const BASE_COMMAND: [&str; 6] = [
    "-p",
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "text",
];
pub(super) const MANAGED_ENV_KEYS: [&str; 6] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CONFIG_DIR",
];
pub(super) const MANAGED_CONFIG_KEYS: [&str; 7] = [
    "env",
    "apiKeyHelper",
    "model",
    "permissions",
    "awsAuthRefresh",
    "awsCredentialExport",
    "forceLoginMethod",
];
pub(super) const STATIC_ENV: [(&str, &str); 4] = [
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("DISABLE_AUTOUPDATER", "1"),
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
];
pub(super) const STRUCTURED_OUTPUT_INSTRUCTION: &str = "When you have finished the task, end your final reply with only a single JSON object (no code fences, no prose after it) that matches this JSON schema:\n{schema}";
pub(super) const PERMISSION_MODES: [(&str, &str); 4] = [
    ("read-only", "plan"),
    ("ask", "default"),
    ("edit", "acceptEdits"),
    ("full", "bypassPermissions"),
];
pub(super) const NATIVE_TO_NORMALIZED: [(&str, &str); 9] = [
    ("Read", "read"),
    ("Write", "write"),
    ("Edit", "edit"),
    ("MultiEdit", "edit"),
    ("Bash", "bash"),
    ("Glob", "glob"),
    ("Grep", "grep"),
    ("WebSearch", "web_search"),
    ("LS", "ls"),
];
pub(super) const NORMALIZED_TO_NATIVE: [(&str, &[&str]); 8] = [
    ("read", &["Read"]),
    ("write", &["Write"]),
    ("edit", &["Edit", "MultiEdit"]),
    ("bash", &["Bash"]),
    ("glob", &["Glob"]),
    ("grep", &["Grep"]),
    ("web_search", &["WebSearch"]),
    ("ls", &["LS"]),
];

pub(super) fn validate_options(options: &ClaudeCodeOptions) -> Result<(), HarnessError> {
    let mut env_conflicts = MANAGED_ENV_KEYS
        .iter()
        .filter(|key| options.env.contains_key(**key))
        .copied()
        .collect::<Vec<_>>();
    if !env_conflicts.is_empty() {
        env_conflicts.sort_unstable();
        return Err(HarnessError::ClaudeCodeManagedEnv {
            keys: env_conflicts.join(", "),
        });
    }
    let mut config_conflicts = MANAGED_CONFIG_KEYS
        .iter()
        .filter(|key| options.config.contains_key(**key))
        .copied()
        .collect::<Vec<_>>();
    if !config_conflicts.is_empty() {
        config_conflicts.sort_unstable();
        return Err(HarnessError::ClaudeCodeManagedConfig {
            keys: config_conflicts.join(", "),
        });
    }
    Ok(())
}

pub(super) fn permission_mode(mode: PermissionMode) -> &'static str {
    let name = match mode {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::Ask => "ask",
        PermissionMode::Edit => "edit",
        PermissionMode::Full => "full",
    };
    PERMISSION_MODES
        .iter()
        .find(|(key, _)| *key == name)
        .map_or(name, |(_, value)| value)
}

pub(super) fn native_tool_names(disabled: &[String]) -> Vec<String> {
    let mapping: BTreeMap<String, Vec<String>> = NORMALIZED_TO_NATIVE
        .iter()
        .map(|(key, native)| {
            (
                (*key).to_owned(),
                native.iter().map(|name| (*name).to_owned()).collect(),
            )
        })
        .collect();
    shared_native_tool_names(disabled, &mapping)
}

pub(super) fn normalize_tool_name(native: &str) -> String {
    match NATIVE_TO_NORMALIZED.iter().find(|(key, _)| *key == native) {
        Some((_, normalized)) => (*normalized).to_owned(),
        None => native.to_owned(),
    }
}

pub(super) fn build_system_prompt(
    instructions: Option<&str>,
    output_schema: Option<&JsonObject>,
) -> Result<Option<String>, HarnessError> {
    let schema_part = output_schema
        .map(|schema| {
            python_json_text(schema)
                .map(|text| STRUCTURED_OUTPUT_INSTRUCTION.replace("{schema}", &text))
        })
        .transpose()
        .map_err(|source| HarnessError::StateEncode { source })?;
    let parts: Vec<&str> = [instructions, schema_part.as_deref()]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
}
