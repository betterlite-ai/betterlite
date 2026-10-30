use std::collections::BTreeMap;

use serde_json::Value;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::payloads::structured_output_instruction;
use crate::types::{Harness, JsonObject, PermissionMode};

use super::permissions::permission_rules;

pub(super) const OPENCODE_BINARY: &str = "opencode";
pub(super) const OPENCODE_PROVIDER_ID: &str = "litellm";
const OPENCODE_PROVIDER_NPM: &str = "@ai-sdk/openai-compatible";
// a fixed title skips opencode's extra title-generation model call on the first turn
pub(super) const OPENCODE_SESSION_TITLE: &str = "litellm-harness";
pub(super) const TOKEN_FILENAME: &str = "token";
pub(super) const INSTRUCTIONS_FILENAME: &str = "instructions.md";
pub(super) const XDG_DIRNAME: &str = "xdg";
pub(super) const XDG_SUBDIRS: [&str; 4] = ["config", "data", "state", "cache"];

// keeps opencode off the network (except the endpoint) and away from ~/.claude
pub(super) const OPENCODE_ISOLATION_ENV: [(&str, &str); 11] = [
    ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
    ("OPENCODE_DISABLE_MODELS_FETCH", "1"),
    ("OPENCODE_DISABLE_LSP_DOWNLOAD", "1"),
    ("OPENCODE_DISABLE_SHARE", "1"),
    ("OPENCODE_DISABLE_DEFAULT_PLUGINS", "1"),
    ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
    ("OPENCODE_DISABLE_EXTERNAL_SKILLS", "1"),
    ("OPENCODE_CONFIG", ""),
    ("OPENCODE_CONFIG_DIR", ""),
    ("OPENCODE_PERMISSION", ""),
    ("OPENCODE_AUTH_CONTENT", ""),
];

const MANAGED_CONFIG_KEYS: [&str; 8] = [
    "provider",
    "model",
    "small_model",
    "permission",
    "tools",
    "enabled_providers",
    "disabled_providers",
    "plugin",
];
const AGENT_MANAGED_KEYS: [&str; 3] = ["permission", "tools", "model"];
const AGENT_SECTIONS: [&str; 2] = ["agent", "mode"];

pub(super) fn validate_user_config(config: &JsonObject) -> Result<(), HarnessError> {
    reject_managed_keys(config)?;
    for section in AGENT_SECTIONS {
        reject_agent_keys(config, section)?;
    }
    Ok(())
}

fn reject_managed_keys(config: &JsonObject) -> Result<(), HarnessError> {
    let managed: Vec<String> = config
        .keys()
        .filter(|key| MANAGED_CONFIG_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    if managed.len() == 1 {
        let Some(key) = managed.first() else {
            return Ok(());
        };
        return Err(HarnessError::ManagedConfigKey {
            key: key.clone(),
            harness: Harness::OpenCode,
        });
    }
    if !managed.is_empty() {
        return Err(HarnessError::ManagedConfigKey {
            key: managed.join(", "),
            harness: Harness::OpenCode,
        });
    }
    Ok(())
}

fn reject_agent_keys(config: &JsonObject, section: &str) -> Result<(), HarnessError> {
    let Some(entries) = config.get(section) else {
        return Ok(());
    };
    let Value::Object(agents) = entries else {
        return Err(HarnessError::OptionsMismatch {
            given: "OpenCodeOptions.config[agent] or [mode]",
            harness: Harness::OpenCode,
            expected: "a mapping of names to agent config",
        });
    };
    for (agent_name, agent) in agents {
        let Value::Object(agent_fields) = agent else {
            return Err(HarnessError::OptionsMismatch {
                given: "OpenCodeOptions.config[agent/mode]",
                harness: Harness::OpenCode,
                expected: "a mapping of names to agent config",
            });
        };
        let mut keys: Vec<String> = agent_fields
            .keys()
            .filter(|key| AGENT_MANAGED_KEYS.contains(&key.as_str()))
            .cloned()
            .collect();
        if !keys.is_empty() {
            keys.sort();
            return Err(HarnessError::ManagedConfigKey {
                key: format!("{section}.{agent_name}: {}", keys.join(", ")),
                harness: Harness::OpenCode,
            });
        }
    }
    Ok(())
}

pub(super) struct ConfigInputs<'cfg> {
    pub(super) model: &'cfg str,
    pub(super) base_url: String,
    pub(super) token_path: String,
    pub(super) permissions: PermissionMode,
    pub(super) disable_tools: &'cfg [String],
    pub(super) user_config: &'cfg JsonObject,
    pub(super) instructions_path: Option<String>,
    pub(super) skills_path: Option<String>,
}

fn provider_block(base_url: &str, token_path: &str, model: &str) -> JsonObject {
    let mut options = JsonObject::new();
    options.insert("baseURL".to_owned(), Value::String(base_url.to_owned()));
    options.insert(
        "apiKey".to_owned(),
        Value::String(format!("{{file:{token_path}}}")),
    );
    let mut models = JsonObject::new();
    models.insert(
        model.to_owned(),
        Value::Object(JsonObject::new().into_iter().collect()),
    );
    let mut provider = JsonObject::new();
    provider.insert(
        "npm".to_owned(),
        Value::String(OPENCODE_PROVIDER_NPM.to_owned()),
    );
    provider.insert("name".to_owned(), Value::String("LiteLLM".to_owned()));
    provider.insert(
        "options".to_owned(),
        Value::Object(options.into_iter().collect()),
    );
    provider.insert(
        "models".to_owned(),
        Value::Object(models.into_iter().collect()),
    );
    let mut providers = JsonObject::new();
    providers.insert(
        OPENCODE_PROVIDER_ID.to_owned(),
        Value::Object(provider.into_iter().collect()),
    );
    providers
}

fn merged_instructions(
    user_config: &JsonObject,
    extra: Option<&str>,
) -> Result<Vec<String>, HarnessError> {
    let mut instructions: Vec<String> =
        if let Some(Value::Array(items)) = user_config.get("instructions") {
            items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or(HarnessError::OptionsMismatch {
                    given: "OpenCodeOptions.config[instructions]",
                    harness: Harness::OpenCode,
                    expected: "a list containing only strings",
                })?
        } else {
            Vec::new()
        };
    if let Some(path) = extra {
        instructions.push(path.to_owned());
    }
    Ok(instructions)
}

fn merged_skills(
    user_config: &JsonObject,
    extra: Option<&str>,
) -> Result<Option<JsonObject>, HarnessError> {
    let mut skills = if let Some(Value::Object(entries)) = user_config.get("skills") {
        entries
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    } else {
        JsonObject::new()
    };
    let mut paths: Vec<String> = if let Some(Value::Array(items)) = skills.get("paths") {
        items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or(HarnessError::OptionsMismatch {
                given: "OpenCodeOptions.config[skills.paths]",
                harness: Harness::OpenCode,
                expected: "a list containing only strings",
            })?
    } else {
        Vec::new()
    };
    if let Some(path) = extra {
        paths.push(path.to_owned());
    }
    if paths.is_empty() {
        return Ok(None);
    }
    skills.insert(
        "paths".to_owned(),
        Value::Array(paths.into_iter().map(Value::String).collect()),
    );
    Ok(Some(skills))
}

fn insert_managed_fields(
    config: &mut JsonObject,
    inputs: &ConfigInputs<'_>,
    permission: BTreeMap<String, String>,
) {
    let qualified_model = format!("{OPENCODE_PROVIDER_ID}/{}", inputs.model);
    config.insert(
        "provider".to_owned(),
        Value::Object(
            provider_block(&inputs.base_url, &inputs.token_path, inputs.model)
                .into_iter()
                .collect(),
        ),
    );
    config.insert(
        "enabled_providers".to_owned(),
        Value::Array(vec![Value::String(OPENCODE_PROVIDER_ID.to_owned())]),
    );
    config.insert("model".to_owned(), Value::String(qualified_model.clone()));
    config.insert("small_model".to_owned(), Value::String(qualified_model));
    config.insert(
        "permission".to_owned(),
        Value::Object(
            permission
                .into_iter()
                .map(|(key, value)| (key, Value::String(value)))
                .collect(),
        ),
    );
    config.insert("autoupdate".to_owned(), Value::Bool(false));
    config.insert("share".to_owned(), Value::String("disabled".to_owned()));
}

pub(super) fn build_opencode_config(inputs: &ConfigInputs<'_>) -> Result<JsonObject, HarnessError> {
    validate_user_config(inputs.user_config)?;
    let instructions =
        merged_instructions(inputs.user_config, inputs.instructions_path.as_deref())?;
    let skills = merged_skills(inputs.user_config, inputs.skills_path.as_deref())?;
    let permission = permission_rules(inputs.permissions, inputs.disable_tools);
    let mut config = inputs.user_config.clone();
    insert_managed_fields(&mut config, inputs, permission);
    if !instructions.is_empty() {
        config.insert(
            "instructions".to_owned(),
            Value::Array(instructions.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(skills_value) = skills {
        config.insert(
            "skills".to_owned(),
            Value::Object(skills_value.into_iter().collect()),
        );
    }
    Ok(config)
}

pub(super) fn build_instructions<S>(
    context: &SessionContext<S>,
) -> Result<Option<String>, HarnessError> {
    let schema_part = match context.settings.output_schema.as_ref() {
        Some(schema) => Some(
            structured_output_instruction(schema)
                .map_err(|source| HarnessError::OutputSchema { source })?,
        ),
        None => None,
    };
    let sections: Vec<&str> = [
        context.settings.instructions.as_deref(),
        schema_part.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|section| !section.is_empty())
    .collect();
    Ok(if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    })
}

pub(super) fn turn_prompt<S>(
    context: &SessionContext<S>,
    prompt: &str,
) -> Result<String, HarnessError> {
    match context.settings.output_schema.as_ref() {
        Some(schema) => Ok(format!(
            "{prompt}\n\n{}",
            structured_output_instruction(schema)
                .map_err(|source| HarnessError::OutputSchema { source })?
        )),
        None => Ok(prompt.to_owned()),
    }
}
