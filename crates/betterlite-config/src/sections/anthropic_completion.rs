use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OptionValues {
    pub max_tokens_to_sample: Option<i64>,
    pub stop_sequences: Option<Vec<String>>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<i64>,
    pub metadata: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub deployments: Vec<OptionValues>,
    pub litellm_settings: OptionValues,
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
    max_tokens_to_sample: Option<RawNumber>,
    stop_sequences: Option<Vec<String>>,
    temperature: Option<RawNumber>,
    top_p: Option<RawNumber>,
    top_k: Option<RawNumber>,
    metadata: Option<IndexMap<String, Json>>,
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
        })
    }
}

fn option_values<L>(raw: RawValues, prefix: &str, lookup: &L) -> Result<OptionValues, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = |key: &str| format!("{prefix}.{key}");
    let max_tokens_to_sample = resolve_number(
        raw.max_tokens_to_sample,
        &path("max_tokens_to_sample"),
        lookup,
        integer,
    )?;
    let temperature = resolve_number(raw.temperature, &path("temperature"), lookup, parse_float)?;
    let top_p = resolve_number(raw.top_p, &path("top_p"), lookup, parse_float)?;
    let top_k = resolve_number(raw.top_k, &path("top_k"), lookup, integer)?;
    let metadata = raw
        .metadata
        .map(|fields| resolved_fields(fields, &path("metadata"), lookup))
        .transpose()?;
    Ok(OptionValues {
        max_tokens_to_sample,
        stop_sequences: raw.stop_sequences,
        temperature,
        top_p,
        top_k,
        metadata,
    })
}

fn resolved_fields<L>(
    fields: IndexMap<String, Json>,
    path: &str,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = IndexMap::with_capacity(fields.len());
    for (key, field) in fields {
        resolved.insert(
            key.clone(),
            resolved_json(field, &format!("{path}.{key}"), lookup)?,
        );
    }
    Ok(resolved)
}

// LiteLLM recurses into objects and the dict items of a list, and leaves every other list item as it is
fn resolved_json<L>(value: Json, path: &str, lookup: &L) -> Result<Json, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => Ok(Json::Text(resolve_string(text, path, lookup)?)),
        Json::Object(fields) => Ok(Json::Object(resolved_fields(fields, path, lookup)?)),
        Json::List(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                resolved.push(match item {
                    Json::Object(fields) => Json::Object(resolved_fields(fields, path, lookup)?),
                    other => other,
                });
            }
            Ok(Json::List(resolved))
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
