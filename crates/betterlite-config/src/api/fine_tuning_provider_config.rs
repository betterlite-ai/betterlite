use betterlite_models::decisions::Json;
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

pub const FINETUNE_SETTINGS: &str = "finetune_settings";

// top-level finetune_settings, kept as written so every lookup sees the provider order
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub finetune_settings: Option<Json>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub finetune_settings: Option<Vec<Json>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.finetune_settings.as_ref();
        let finetune_settings =
            set_fine_tuning_config(None, raw, lookup).map_err(|error| error.into_config(raw))?;
        Ok(Settings { finetune_settings })
    }
}

#[derive(Debug, Error)]
pub enum FineTuningConfigError {
    #[error("fine_tuning_config is not set, set it on your config.yaml file.")]
    Unset,
    #[error("invalid fine_tuning config, expected a list is not a list")]
    NotAList,
    // Python reaches a non-dict entry with .get, so the lookup fails instead of skipping it
    #[error("'{kind}' object has no attribute 'get'")]
    NotAProviderMap { kind: &'static str },
    #[error(transparent)]
    Environment(#[from] ConfigError),
}

impl FineTuningConfigError {
    // the loader reports the key's path, and its bool flag only when the value written was a true
    fn into_config(self, raw: Option<&Json>) -> ConfigError {
        match self {
            Self::Environment(error) => error,
            Self::Unset | Self::NotAList | Self::NotAProviderMap { .. } => {
                ConfigError::InvalidScalar {
                    path: FINETUNE_SETTINGS.to_owned(),
                    value: matches!(raw, Some(Json::Bool(true))),
                }
            }
        }
    }
}

// set_fine_tuning_config: None keeps the current list, a list stores its shallow os.environ/ strings resolved
pub fn set_fine_tuning_config<L>(
    current: Option<Vec<Json>>,
    config: Option<&Json>,
    lookup: &L,
) -> Result<Option<Vec<Json>>, FineTuningConfigError>
where
    L: EnvironmentLookup,
{
    let items = match config {
        None | Some(Json::Null) => return Ok(current),
        Some(Json::List(items)) => items,
        Some(_) => return Err(FineTuningConfigError::NotAList),
    };
    let mut settings = Vec::with_capacity(items.len());
    for item in items {
        settings.push(match item {
            Json::Object(entries) => {
                let mut resolved = entries.clone();
                for value in resolved.values_mut() {
                    if let Json::Text(text) = value
                        && text.starts_with("os.environ/")
                    {
                        *value =
                            Json::Text(resolve_string(text.clone(), FINETUNE_SETTINGS, lookup)?);
                    }
                }
                Json::Object(resolved)
            }
            other => other.clone(),
        });
    }
    Ok(Some(settings))
}

// fine_tuning_config_for: the whole first entry whose custom_llm_provider matches
pub fn fine_tuning_config_for(
    current: Option<&[Json]>,
    provider: &str,
) -> Result<Option<Json>, FineTuningConfigError> {
    let Some(settings) = current else {
        return Err(FineTuningConfigError::Unset);
    };
    for setting in settings {
        let Json::Object(entries) = setting else {
            return Err(FineTuningConfigError::NotAProviderMap {
                kind: value_kind(setting),
            });
        };
        let matched = entries
            .get("custom_llm_provider")
            .is_some_and(|value| matches!(value, Json::Text(text) if text == provider));
        if matched {
            return Ok(Some(setting.clone()));
        }
    }
    Ok(None)
}

// the type name Python puts in the AttributeError message
fn value_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Number(number) if number.is_f64() => "float",
        Json::Number(_) => "int",
        Json::Text(_) => "str",
        Json::List(_) => "list",
        Json::Object(_) => "dict",
    }
}
