use std::collections::BTreeMap;

use serde_json::json;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::payloads::structured_output_instruction;
use crate::types::{Harness, JsonObject, PermissionMode};

use super::permissions::permission_rules;
pub(super) use super::validation::validate_user_config;

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
    options.insert("baseURL".to_owned(), json!(base_url));
    options.insert("apiKey".to_owned(), json!(format!("{{file:{token_path}}}")));
    let mut models = JsonObject::new();
    models.insert(
        model.to_owned(),
        serde_json::to_value(JsonObject::new()).unwrap_or_default(),
    );
    let mut provider = JsonObject::new();
    provider.insert("npm".to_owned(), json!(OPENCODE_PROVIDER_NPM));
    provider.insert("name".to_owned(), json!("LiteLLM"));
    provider.insert("options".to_owned(), json!(options));
    provider.insert("models".to_owned(), json!(models));
    let mut providers = JsonObject::new();
    providers.insert(OPENCODE_PROVIDER_ID.to_owned(), json!(provider));
    providers
}

fn string_entries(
    config: &JsonObject,
    key: &str,
    given: &'static str,
) -> Result<Vec<String>, HarnessError> {
    let Some(items) = config.get(key).and_then(|value| value.as_array()) else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .map(|item| item.as_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()
        .ok_or(HarnessError::OptionsMismatch {
            given,
            harness: Harness::OpenCode,
            expected: "a list containing only strings",
        })
}

fn merged_instructions(
    user_config: &JsonObject,
    extra: Option<&str>,
) -> Result<Vec<String>, HarnessError> {
    let mut instructions = string_entries(
        user_config,
        "instructions",
        "OpenCodeOptions.config[instructions]",
    )?;
    if let Some(path) = extra {
        instructions.push(path.to_owned());
    }
    Ok(instructions)
}

fn merged_skills(
    user_config: &JsonObject,
    extra: Option<&str>,
) -> Result<Option<JsonObject>, HarnessError> {
    let mut skills: JsonObject = user_config
        .get("skills")
        .and_then(|value| value.as_object())
        .map(|entries| entries.clone().into_iter().collect())
        .unwrap_or_default();
    let mut paths = string_entries(&skills, "paths", "OpenCodeOptions.config[skills.paths]")?;
    if let Some(path) = extra {
        paths.push(path.to_owned());
    }
    if paths.is_empty() {
        return Ok(None);
    }
    skills.insert("paths".to_owned(), json!(paths));
    Ok(Some(skills))
}

fn insert_managed_fields(
    config: &mut JsonObject,
    inputs: &ConfigInputs<'_>,
    permission: &BTreeMap<String, String>,
) {
    let qualified_model = format!("{OPENCODE_PROVIDER_ID}/{}", inputs.model);
    config.insert(
        "provider".to_owned(),
        json!(provider_block(
            &inputs.base_url,
            &inputs.token_path,
            inputs.model
        )),
    );
    config.insert(
        "enabled_providers".to_owned(),
        json!([OPENCODE_PROVIDER_ID]),
    );
    let small_model = qualified_model.clone();
    config.insert("model".to_owned(), json!(qualified_model));
    config.insert("small_model".to_owned(), json!(small_model));
    config.insert("permission".to_owned(), json!(permission));
    config.insert("autoupdate".to_owned(), json!(false));
    config.insert("share".to_owned(), json!("disabled"));
}

pub(super) fn build_opencode_config(inputs: &ConfigInputs<'_>) -> Result<JsonObject, HarnessError> {
    validate_user_config(inputs.user_config)?;
    let instructions =
        merged_instructions(inputs.user_config, inputs.instructions_path.as_deref())?;
    let skills = merged_skills(inputs.user_config, inputs.skills_path.as_deref())?;
    let permission = permission_rules(inputs.permissions, inputs.disable_tools);
    let mut config = inputs.user_config.clone();
    insert_managed_fields(&mut config, inputs, &permission);
    if !instructions.is_empty() {
        config.insert("instructions".to_owned(), json!(instructions));
    }
    if let Some(skills_value) = skills {
        config.insert("skills".to_owned(), json!(skills_value));
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
