use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use crate::opencode::{ListedModel, chat_models};

pub const CODEX_BASE_INSTRUCTIONS: &str = include_str!("codex_base_instructions.md");

const LIST_VISIBILITY: &str = "list";

// clippy's max-struct-bools counts plain bools, so the wire flags share the alias the other crates use
pub type CodexFlag = bool;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CodexTruncationPolicy {
    #[serde(default)]
    pub mode: CodexTruncationMode,
    #[serde(default = "default_limit")]
    pub limit: u32,
}

impl Default for CodexTruncationPolicy {
    fn default() -> Self {
        Self {
            mode: CodexTruncationMode::Bytes,
            limit: default_limit(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum CodexTruncationMode {
    #[default]
    #[serde(rename = "bytes")]
    Bytes,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum CodexShellType {
    #[default]
    #[serde(rename = "unified_exec")]
    UnifiedExec,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub enum CodexVisibility {
    #[default]
    #[serde(rename = "list")]
    List,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodexModel {
    pub slug: String,
    pub display_name: String,
    #[serde(default)]
    pub description: (),
    #[serde(default)]
    pub supported_reasoning_levels: [JsonValue; 0],
    #[serde(default)]
    pub shell_type: CodexShellType,
    #[serde(default)]
    pub visibility: CodexVisibility,
    #[serde(default = "default_supported_in_api")]
    pub supported_in_api: CodexFlag,
    pub priority: i64,
    #[serde(default)]
    pub availability_nux: (),
    #[serde(default)]
    pub upgrade: (),
    #[serde(default)]
    pub support_verbosity: CodexFlag,
    #[serde(default)]
    pub supports_reasoning_summaries: CodexFlag,
    #[serde(default)]
    pub supports_parallel_tool_calls: CodexFlag,
    #[serde(default)]
    pub default_verbosity: (),
    #[serde(default)]
    pub apply_patch_tool_type: (),
    #[serde(default)]
    pub truncation_policy: CodexTruncationPolicy,
    #[serde(default)]
    pub experimental_supported_tools: [JsonValue; 0],
    pub context_window: Option<i64>,
    pub base_instructions: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StockCodexUpgrade {
    pub model: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StockCodexModel {
    pub slug: String,
    pub priority: i64,
    pub visibility: String,
    #[serde(default = "default_supported_in_api")]
    pub supported_in_api: CodexFlag,
    #[serde(default)]
    pub upgrade: Option<StockCodexUpgrade>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonValue>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct StockCodexCatalog {
    pub models: Vec<StockCodexModel>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CodexCatalogModel {
    Known(StockCodexModel),
    Fallback(CodexModel),
}

#[derive(Serialize)]
struct CodexCatalog {
    models: Vec<CodexCatalogModel>,
}

#[must_use]
pub fn codex_catalog_entry(
    priority: i64,
    listed: &ListedModel,
    stock: Option<&StockCodexModel>,
    served: &BTreeSet<String>,
    instructions: &str,
) -> CodexCatalogModel {
    let Some(installed) = stock else {
        return CodexCatalogModel::Fallback(CodexModel {
            slug: listed.id.clone(),
            display_name: listed.id.clone(),
            description: (),
            supported_reasoning_levels: [],
            shell_type: CodexShellType::UnifiedExec,
            visibility: CodexVisibility::List,
            supported_in_api: true,
            priority,
            availability_nux: (),
            upgrade: (),
            support_verbosity: false,
            supports_reasoning_summaries: false,
            supports_parallel_tool_calls: false,
            default_verbosity: (),
            apply_patch_tool_type: (),
            truncation_policy: CodexTruncationPolicy::default(),
            experimental_supported_tools: [],
            context_window: listed.max_input_tokens,
            base_instructions: instructions.to_owned(),
        });
    };
    let upgrade = installed
        .upgrade
        .clone()
        .filter(|upgrade| served.contains(&upgrade.model));
    CodexCatalogModel::Known(StockCodexModel {
        priority,
        visibility: LIST_VISIBILITY.to_owned(),
        supported_in_api: true,
        upgrade,
        ..installed.clone()
    })
}

#[must_use]
pub fn codex_model_catalog(
    models: &[ListedModel],
    stock: &[StockCodexModel],
    instructions: &str,
) -> Option<String> {
    let chat = chat_models(models);
    if chat.is_empty() {
        return None;
    }
    let served: BTreeSet<String> = chat.iter().map(|model| model.id.clone()).collect();
    let known: BTreeMap<&str, &StockCodexModel> = stock
        .iter()
        .map(|model| (model.slug.as_str(), model))
        .collect();
    let catalog = CodexCatalog {
        models: (0_i64..)
            .zip(chat.iter())
            .map(|(priority, model)| {
                codex_catalog_entry(
                    priority,
                    model,
                    known.get(model.id.as_str()).copied(),
                    &served,
                    instructions,
                )
            })
            .collect(),
    };
    // the catalog holds only strings, numbers, flags and JSON passthrough values, so encoding cannot fail
    serde_json::to_string(&catalog).ok()
}

const fn default_limit() -> u32 {
    10_000
}

const fn default_supported_in_api() -> CodexFlag {
    true
}

mod tests;
