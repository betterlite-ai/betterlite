use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const PARAMS_PATH: &str = "litellm_settings.compression_interception_params";
const CALLBACK_PATH: &str = "callback_settings.compression_interception";
const DEFAULT_TRIGGER: usize = 200_000;
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlmSettings,
    #[serde(default)]
    callback_settings: RawCallbackSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlmSettings {
    #[serde(default)]
    compression_interception_params: Option<RawParams>,
}

#[derive(Debug, Default, Deserialize)]
struct RawCallbackSettings {
    #[serde(default)]
    compression_interception: Option<RawParams>,
}

#[derive(Debug, Default, Deserialize)]
struct RawParams {
    #[serde(default)]
    enabled: Option<RawFlag>,
    #[serde(default)]
    compression_trigger: Option<RawNumber>,
    #[serde(default)]
    compression_target: Option<RawNumber>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    embedding_model_params: Option<IndexMap<String, Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub compression_trigger: usize,
    pub compression_target: Option<usize>,
    pub embedding_model: Option<String>,
    pub embedding_model_params: Option<IndexMap<String, Json>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            compression_trigger: DEFAULT_TRIGGER,
            compression_target: None,
            embedding_model: None,
            embedding_model_params: None,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // LiteLLM's initialize_from_proxy_config prefers the top-level params and falls back to the callback's entry
        if let Some(params) = self.litellm_settings.compression_interception_params {
            return convert_params(params, PARAMS_PATH, lookup);
        }
        self.callback_settings.compression_interception.map_or_else(
            || Ok(Settings::default()),
            |params| convert_params(params, CALLBACK_PATH, lookup),
        )
    }
}

fn convert_params<L>(params: RawParams, path: &str, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    let enabled = match params.enabled {
        None => true,
        Some(RawFlag::Boolean(flag)) => flag,
        Some(RawFlag::Text(text)) => {
            let field = format!("{path}.enabled");
            let name = text
                .strip_prefix(ENVIRONMENT_PREFIX)
                .unwrap_or(&field)
                .to_owned();
            let resolved = resolve_string(text, &field, lookup)?;
            betterlite_env::flag(&name, &resolved)?.unwrap_or(true)
        }
    };
    let compression_trigger = resolve_number(
        params.compression_trigger,
        &format!("{path}.compression_trigger"),
        lookup,
        parse_count,
    )?
    .unwrap_or(DEFAULT_TRIGGER);
    let compression_target = resolve_number(
        params.compression_target,
        &format!("{path}.compression_target"),
        lookup,
        parse_count,
    )?;
    let embedding_model = optional_text(
        params.embedding_model,
        &format!("{path}.embedding_model"),
        lookup,
    )?;
    Ok(Settings {
        enabled,
        compression_trigger,
        compression_target,
        embedding_model,
        embedding_model_params: params.embedding_model_params,
    })
}

fn optional_text<L>(
    value: Option<String>,
    path: &str,
    lookup: &L,
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = value else {
        return Ok(None);
    };
    let from_environment = text.starts_with(ENVIRONMENT_PREFIX);
    let resolved = resolve_string(text, path, lookup)?;
    if from_environment && resolved.is_empty() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

fn parse_count(text: &str, path: &str) -> Result<usize, ConfigError> {
    text.parse::<usize>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}
