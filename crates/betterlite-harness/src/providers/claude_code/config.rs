use crate::errors::HarnessError;
use crate::options::ClaudeCodeOptions;
use crate::types::{Harness, PermissionMode};

pub const CLAUDE_BINARY: &str = "claude";
pub const BASE_COMMAND: [&str; 6] = [
    "-p",
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "text",
];
pub const MANAGED_ENV_KEYS: [&str; 6] = [
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "CLAUDE_CONFIG_DIR",
];
pub const MANAGED_CONFIG_KEYS: [&str; 7] = [
    "env",
    "apiKeyHelper",
    "model",
    "permissions",
    "awsAuthRefresh",
    "awsCredentialExport",
    "forceLoginMethod",
];
pub const STATIC_ENV: [(&str, &str); 4] = [
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
    ("DISABLE_AUTOUPDATER", "1"),
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
];
pub const STRUCTURED_OUTPUT_INSTRUCTION: &str = "When you have finished the task, end your final reply with only a single JSON object (no code fences, no prose after it) that matches this JSON schema:\n{schema}";
pub const PERMISSION_MODES: [(&str, &str); 4] = [
    ("read-only", "plan"),
    ("ask", "default"),
    ("edit", "acceptEdits"),
    ("full", "bypassPermissions"),
];
pub const NATIVE_TO_NORMALIZED: [(&str, &str); 9] = [
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
pub const NORMALIZED_TO_NATIVE: [(&str, &[&str]); 8] = [
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
    if MANAGED_ENV_KEYS
        .iter()
        .any(|key| options.env.contains_key(*key))
    {
        return Err(HarnessError::OptionsMismatch {
            given: "ClaudeCodeOptions.env",
            harness: Harness::ClaudeCode,
            expected: "keys other than managed Claude Code environment keys",
        });
    }
    if let Some(key) = MANAGED_CONFIG_KEYS
        .iter()
        .find(|key| options.config.keys().any(|candidate| candidate == *key))
    {
        return Err(HarnessError::ManagedConfigKey {
            key: (*key).to_owned(),
            harness: Harness::ClaudeCode,
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
    let mut names = Vec::new();
    for normalized in disabled {
        if let Some((_, native)) = NORMALIZED_TO_NATIVE
            .iter()
            .find(|(key, _)| *key == normalized)
        {
            names.extend(native.iter().map(|name| (*name).to_owned()));
        } else {
            names.push(normalized.clone());
        }
    }
    names.sort();
    names.dedup();
    names
}

pub(super) fn normalize_tool_name(native: &str) -> String {
    let normalized = NATIVE_TO_NORMALIZED
        .iter()
        .find(|(key, _)| *key == native)
        .map_or_else(
            || native.to_owned(),
            |(_, normalized)| (*normalized).to_owned(),
        );
    normalized
}
