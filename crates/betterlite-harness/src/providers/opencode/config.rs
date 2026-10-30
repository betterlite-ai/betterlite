use std::collections::BTreeMap;

use serde::Serialize;

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

#[derive(Serialize)]
struct ProviderBlock {
    npm: &'static str,
    name: &'static str,
    options: ProviderOptions,
    models: BTreeMap<String, ModelConfig>,
}

#[derive(Serialize)]
struct ProviderOptions {
    #[serde(rename = "baseURL")]
    base_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
}

#[derive(Serialize)]
struct ModelConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<()>,
}

#[derive(Serialize)]
struct SkillsConfig {
    paths: Vec<String>,
    #[serde(flatten)]
    extra: JsonObject,
}

#[derive(Serialize)]
struct OpenCodeConfig {
    provider: BTreeMap<String, ProviderBlock>,
    enabled_providers: Vec<&'static str>,
    model: String,
    small_model: String,
    permission: BTreeMap<String, String>,
    autoupdate: bool,
    share: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills: Option<SkillsConfig>,
    #[serde(flatten)]
    extra: JsonObject,
}

fn provider_block(
    base_url: &str,
    token_path: &str,
    model: &str,
) -> BTreeMap<String, ProviderBlock> {
    let mut models = BTreeMap::new();
    models.insert(model.to_owned(), ModelConfig { options: None });
    BTreeMap::from([(
        OPENCODE_PROVIDER_ID.to_owned(),
        ProviderBlock {
            npm: OPENCODE_PROVIDER_NPM,
            name: "LiteLLM",
            options: ProviderOptions {
                base_url: base_url.to_owned(),
                api_key: format!("{{file:{token_path}}}"),
            },
            models,
        },
    )])
}

fn string_entries(
    config: &JsonObject,
    key: &str,
    given: &'static str,
) -> Result<Vec<String>, HarnessError> {
    let Some(value) = config.get(key) else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err(HarnessError::OptionsMismatch {
            given,
            harness: Harness::OpenCode,
            expected: "a list containing only strings",
        });
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
) -> Result<Option<SkillsConfig>, HarnessError> {
    let skills: JsonObject = match user_config.get("skills") {
        None => JsonObject::new(),
        Some(value) => value
            .as_object()
            .cloned()
            .map(IntoIterator::into_iter)
            .map(Iterator::collect)
            .ok_or(HarnessError::OptionsMismatch {
                given: "OpenCodeOptions.config[skills]",
                harness: Harness::OpenCode,
                expected: "a mapping",
            })?,
    };
    let mut paths = string_entries(&skills, "paths", "OpenCodeOptions.config[skills.paths]")?;
    if let Some(path) = extra {
        paths.push(path.to_owned());
    }
    if paths.is_empty() {
        return Ok(None);
    }
    let skill_extra = skills
        .into_iter()
        .filter(|(key, _)| key != "paths")
        .collect();
    Ok(Some(SkillsConfig {
        paths,
        extra: skill_extra,
    }))
}

fn build_typed_config(
    inputs: &ConfigInputs<'_>,
    permission: &BTreeMap<String, String>,
) -> OpenCodeConfig {
    let qualified_model = format!("{OPENCODE_PROVIDER_ID}/{}", inputs.model);
    let small_model = qualified_model.clone();
    OpenCodeConfig {
        provider: provider_block(&inputs.base_url, &inputs.token_path, inputs.model),
        enabled_providers: vec![OPENCODE_PROVIDER_ID],
        model: qualified_model,
        small_model,
        permission: permission.clone(),
        autoupdate: false,
        share: "disabled",
        instructions: None,
        skills: None,
        extra: inputs.user_config.clone(),
    }
}

pub(super) fn build_opencode_config(inputs: &ConfigInputs<'_>) -> Result<JsonObject, HarnessError> {
    validate_user_config(inputs.user_config)?;
    let instructions =
        merged_instructions(inputs.user_config, inputs.instructions_path.as_deref())?;
    let skills = merged_skills(inputs.user_config, inputs.skills_path.as_deref())?;
    let permission = permission_rules(inputs.permissions, inputs.disable_tools);
    let mut config = build_typed_config(inputs, &permission);
    config.instructions = (!instructions.is_empty()).then_some(instructions);
    config.skills = skills;
    if config.instructions.is_some() {
        config.extra.remove("instructions");
    }
    if config.skills.is_some() {
        config.extra.remove("skills");
    }
    serde_json::to_value(config)
        .map_err(|source| HarnessError::StateEncode { source })?
        .as_object()
        .cloned()
        .map(|object| object.into_iter().collect())
        .ok_or_else(|| HarnessError::OptionsMismatch {
            given: "OpenCodeOptions.config",
            harness: Harness::OpenCode,
            expected: "a JSON object",
        })
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
