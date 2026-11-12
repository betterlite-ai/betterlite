use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

pub const DEFAULT_CHAT_MAX_TOKENS_ENV: &str = "DEFAULT_ANTHROPIC_CHAT_MAX_TOKENS";
pub const DEFAULT_CHAT_MAX_TOKENS: i64 = 4096;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptionValues {
    pub max_tokens: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub metadata: Option<IndexMap<String, Json>>,
    pub system: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub deployments: Vec<OptionValues>,
    pub litellm_settings: OptionValues,
    pub default_chat_max_tokens: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            deployments: Vec::new(),
            litellm_settings: OptionValues::default(),
            default_chat_max_tokens: DEFAULT_CHAT_MAX_TOKENS,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: RawValues,
}

#[derive(Debug, Default, Deserialize)]
struct RawValues {
    max_tokens: Option<RawNumber>,
    stop_sequences: Option<Vec<String>>,
    temperature: Option<RawNumber>,
    top_p: Option<RawNumber>,
    top_k: Option<RawNumber>,
    metadata: Option<IndexMap<String, Json>>,
    system: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut deployments = Vec::with_capacity(self.model_list.len());
        for (index, deployment) in self.model_list.into_iter().enumerate() {
            let prefix = format!("model_list.{index}.litellm_params");
            deployments.push(option_values(deployment.litellm_params, &prefix, lookup)?);
        }
        Ok(Settings {
            deployments,
            litellm_settings: option_values(self.litellm_settings, "litellm_settings", lookup)?,
            default_chat_max_tokens: default_chat_max_tokens(lookup)?,
        })
    }
}

// LiteLLM reads the constant with int(os.getenv(...)) at import, so a non-numeric value stops the load
fn default_chat_max_tokens<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(DEFAULT_CHAT_MAX_TOKENS_ENV) else {
        return Ok(DEFAULT_CHAT_MAX_TOKENS);
    };
    Ok(betterlite_env::number(DEFAULT_CHAT_MAX_TOKENS_ENV, &text)?
        .unwrap_or(DEFAULT_CHAT_MAX_TOKENS))
}

fn option_values<L>(raw: RawValues, prefix: &str, lookup: &L) -> Result<OptionValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let max_tokens = resolve_number(
        raw.max_tokens,
        &format!("{prefix}.max_tokens"),
        lookup,
        integer,
    )?;
    let temperature = resolve_number(
        raw.temperature,
        &format!("{prefix}.temperature"),
        lookup,
        parse_float,
    )?;
    let top_p = resolve_number(raw.top_p, &format!("{prefix}.top_p"), lookup, parse_float)?;
    let top_k = resolve_number(raw.top_k, &format!("{prefix}.top_k"), lookup, integer)?;
    let system = raw
        .system
        .map(|text| resolve_string(text, &format!("{prefix}.system"), lookup))
        .transpose()?;
    Ok(OptionValues {
        max_tokens,
        stop_sequences: raw.stop_sequences,
        temperature,
        top_p,
        top_k,
        metadata: metadata_values(raw.metadata, prefix, lookup)?,
        system,
    })
}

fn metadata_values<L>(
    value: Option<IndexMap<String, Json>>,
    prefix: &str,
    lookup: &L,
) -> Result<Option<IndexMap<String, Json>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(fields) = value else {
        return Ok(None);
    };
    let base = format!("{prefix}.metadata");
    let mut resolved = IndexMap::with_capacity(fields.len());
    for (key, field) in fields {
        let path = format!("{base}.{key}");
        resolved.insert(key, resolved_json(field, &path, lookup)?);
    }
    Ok(Some(resolved))
}

// LiteLLM recurses into objects and the dict items of a list, and leaves every other list item as it is
fn resolved_json<L>(value: Json, path: &str, lookup: &L) -> Result<Json, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => Ok(Json::Text(resolve_string(text, path, lookup)?)),
        Json::List(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                if let Json::Object(fields) = item {
                    resolved.push(resolved_json(Json::Object(fields), path, lookup)?);
                } else {
                    resolved.push(item);
                }
            }
            Ok(Json::List(resolved))
        }
        Json::Object(fields) => {
            let mut resolved = IndexMap::with_capacity(fields.len());
            for (key, item) in fields {
                resolved.insert(key, resolved_json(item, path, lookup)?);
            }
            Ok(Json::Object(resolved))
        }
        Json::Null | Json::Bool(_) | Json::Number(_) => Ok(value),
    }
}

fn integer(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
