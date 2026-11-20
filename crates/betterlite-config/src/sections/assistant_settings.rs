mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use serde::de;
use serde::{Deserialize, Deserializer};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// proxy_server.py#L6937 expands only litellm_params values that start with this prefix, once, at startup
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub assistant_settings: Option<RawSettings>,
}

// AssistantsTypedDict is a runtime dict, so the loader keeps the parameters as written and checks only
// the two keys the startup block itself reads.
#[derive(Debug, Default)]
pub struct RawSettings {
    custom_llm_provider: Setting<String>,
    litellm_params: Option<BTreeMap<String, Json>>,
}

#[derive(Deserialize)]
struct Fields {
    #[serde(default)]
    custom_llm_provider: Setting<String>,
    #[serde(default)]
    litellm_params: Option<BTreeMap<String, Json>>,
}

impl<'de> Deserialize<'de> for RawSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = Fields::deserialize(deserializer)?;
        // `if assistant_settings:` is false for an empty mapping, so it configures no assistant at all
        if fields.custom_llm_provider.is_absent() && fields.litellm_params.is_none() {
            return Ok(Self::default());
        }
        // once the mapping is nonempty, startup reads assistant_settings["litellm_params"]
        let Some(litellm_params) = fields.litellm_params else {
            return Err(de::Error::missing_field("litellm_params"));
        };
        Ok(Self {
            custom_llm_provider: fields.custom_llm_provider,
            litellm_params: Some(litellm_params),
        })
    }
}

// A field's three YAML states: absent, present with null, present with a value. Serde's Option already
// folds null into absent, but the startup block tests the mapping, not the value, for emptiness.
#[derive(Debug, Default)]
enum Setting<T> {
    #[default]
    Absent,
    Null,
    Value(T),
}

impl<'de, T> Deserialize<'de> for Setting<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<T>::deserialize(deserializer).map_or(Self::Null, Self::Value)
    }
}

impl<T> Setting<T> {
    const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    fn into_option(self) -> Option<T> {
        match self {
            Self::Absent | Self::Null => None,
            Self::Value(value) => Some(value),
        }
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
        let Some(litellm_params) = litellm_params else {
            return Ok(Settings::default());
        };
        let config = Config {
            custom_llm_provider: custom_llm_provider.into_option(),
            litellm_params: Some(expand(litellm_params, lookup)),
        };
        Ok(Settings {
            config: Some(config),
        })
    }
}

fn expand<L>(params: BTreeMap<String, Json>, lookup: &L) -> BTreeMap<String, Json>
where
    L: EnvironmentLookup,
{
    params
        .into_iter()
        .map(|(name, value)| (name, expand_value(value, lookup)))
        .collect()
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
        other => other,
    }
}
