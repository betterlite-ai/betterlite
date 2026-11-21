mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::Deserialize;
use thiserror::Error;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// proxy_server.py#L6937 expands only litellm_params values that start with this prefix, once, at startup
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub assistant_settings: Option<RawSettings>,
}

// AssistantsTypedDict is a runtime dict, so the loader keeps the parameters as written and checks only the two keys startup reads.
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "Fields")]
pub struct RawSettings {
    custom_llm_provider: Option<String>,
    litellm_params: Option<BTreeMap<String, Json>>,
}

// a provider's three YAML states: absent (None), present with null (Some(None)), present with a value
type OptionalProvider = Option<Option<String>>;

#[derive(Deserialize)]
struct Fields {
    #[serde(default, with = "serde_with::rust::double_option")]
    custom_llm_provider: OptionalProvider,
    #[serde(default)]
    litellm_params: Option<BTreeMap<String, Json>>,
}

// the one key the startup block reads once the mapping is nonempty
#[derive(Debug, Error)]
#[error("missing field `litellm_params`")]
struct MissingParams;

impl TryFrom<Fields> for RawSettings {
    type Error = MissingParams;

    fn try_from(fields: Fields) -> Result<Self, Self::Error> {
        let custom_llm_provider = fields.custom_llm_provider;
        // `if assistant_settings:` is false for an empty mapping, so it configures no assistant at all
        if custom_llm_provider.is_none() && fields.litellm_params.is_none() {
            return Ok(Self::default());
        }
        // once the mapping is nonempty, startup reads assistant_settings["litellm_params"]
        let Some(litellm_params) = fields.litellm_params else {
            return Err(MissingParams);
        };
        Ok(Self {
            custom_llm_provider: custom_llm_provider.flatten(),
            litellm_params: Some(litellm_params),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // None: Router(assistants_config=None), so a null provider raises the factory's Exception
    pub config: Option<Config>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    // None: the factory reads the key and raises KeyError('custom_llm_provider')
    pub custom_llm_provider: Option<String>,
    // None only on a directly built assistants_config; the startup loader rejects that mapping
    pub litellm_params: Option<BTreeMap<String, Json>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let Some(RawSettings {
            custom_llm_provider,
            litellm_params,
        }) = self.assistant_settings
        else {
            return Ok(Settings::default());
        };
        let Some(params) = litellm_params else {
            return Ok(Settings::default());
        };
        Ok(Settings {
            config: Some(Config {
                custom_llm_provider,
                litellm_params: Some(
                    params
                        .into_iter()
                        .map(|(name, value)| (name, expand_value(value, lookup)))
                        .collect(),
                ),
            }),
        })
    }
}

// only a top-level string is expanded; a missing variable becomes null and an empty one stays ""
fn expand_value<L>(value: Json, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    match value {
        Json::Text(text) => {
            let Some(rest) = text.strip_prefix(ENVIRONMENT_PREFIX) else {
                return Json::Text(text);
            };
            let variable = rest.replace(ENVIRONMENT_PREFIX, "");
            lookup.lookup(&variable).map_or(Json::Null, Json::Text)
        }
        other @ Json::Null
        | other @ Json::Bool(_)
        | other @ Json::Number(_)
        | other @ Json::List(_)
        | other @ Json::Object(_) => other,
    }
}
