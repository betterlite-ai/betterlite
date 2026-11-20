#[path = "team_fields/tests.rs"]
mod tests;

pub mod schema;

mod timeout;

use std::fmt::Display;
use std::time::Duration;

use serde::Deserialize;
use serde::de::{Error as DeserializeError, IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

use self::schema::{MetadataSchemaField, parse_team_metadata_schema};
use self::timeout::resolve_timeout;

pub const VALIDATOR_SETTING: &str = "custom_team_metadata_validate";
pub const SCHEMA_SETTING: &str = "team_metadata_schema";
pub const TIMEOUT_SETTING: &str = "team_metadata_validation_timeout";
pub const MESSAGE_SETTING: &str = "team_metadata_validation_error_message";

const VALIDATOR_PATH: &str = "general_settings.custom_team_metadata_validate";
const SCHEMA_PATH: &str = "general_settings.team_metadata_schema";
const TIMEOUT_PATH: &str = "general_settings.team_metadata_validation_timeout";
const MESSAGE_PATH: &str = "general_settings.team_metadata_validation_error_message";

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_UNAVAILABLE_MESSAGE: &str = "Team metadata validation is currently unavailable, so the team was not saved. Contact your proxy admin.";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "custom_team_metadata_validate")]
    validator: Option<RawScalar>,
    #[serde(rename = "team_metadata_schema")]
    schema: Option<JsonValue>,
    #[serde(rename = "team_metadata_validation_timeout")]
    timeout: Option<RawScalar>,
    #[serde(rename = "team_metadata_validation_error_message")]
    message: Option<RawScalar>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawScalar {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug)]
enum ResolvedScalar {
    Missing,
    Text(String),
    Other,
}

impl ResolvedScalar {
    // get_instance_fn reads the name off a str, so any other shape fails the config load.
    fn required(self, path: &str, message: &str) -> Result<Option<String>, ConfigError> {
        match self {
            Self::Missing => Ok(None),
            Self::Text(text) if !text.is_empty() => Ok(Some(text)),
            Self::Text(_) | Self::Other => Err(yaml_error(path, message)),
        }
    }

    // A nonblank string is used as written, spaces included; empty, blank and other shapes keep the default.
    fn or_default(self, default: &str) -> String {
        match self {
            Self::Text(text) if !text.trim().is_empty() => text,
            Self::Missing | Self::Text(_) | Self::Other => default.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub validator: Option<String>,
    pub schema: Vec<MetadataSchemaField>,
    pub timeout: ValidationTimeout,
    pub unavailable_message: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            validator: None,
            schema: Vec::new(),
            timeout: ValidationTimeout::Finite(DEFAULT_TIMEOUT),
            unavailable_message: DEFAULT_UNAVAILABLE_MESSAGE.to_owned(),
        }
    }
}

// Positive infinity means the callback runs without a deadline; every other accepted value is a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationTimeout {
    Finite(Duration),
    Unlimited,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        Ok(Settings {
            validator: scalar_text(general.validator, VALIDATOR_PATH, lookup)?.required(
                VALIDATOR_PATH,
                "the value must be a string naming a callable",
            )?,
            schema: parse_team_metadata_schema(general.schema.as_ref())
                .map_err(|source| yaml_error(SCHEMA_PATH, source))?,
            timeout: resolve_timeout(general.timeout, lookup)?,
            unavailable_message: scalar_text(general.message, MESSAGE_PATH, lookup)?
                .or_default(DEFAULT_UNAVAILABLE_MESSAGE),
        })
    }
}

// Text resolves an os.environ/ reference; every other shape stays distinguishable for each setting.
fn scalar_text<L>(
    raw: Option<RawScalar>,
    path: &str,
    lookup: &L,
) -> Result<ResolvedScalar, ConfigError>
where
    L: EnvironmentLookup,
{
    match raw {
        None => Ok(ResolvedScalar::Missing),
        Some(RawScalar::Text(text)) => {
            Ok(ResolvedScalar::Text(resolve_string(text, path, lookup)?))
        }
        Some(_) => Ok(ResolvedScalar::Other),
    }
}

// Only an os.environ/ reference turns text into a number; a written string reaches LiteLLM as a string.
fn environment_seconds<L>(text: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(name) = text.strip_prefix("os.environ/") else {
        return Ok(None);
    };
    let Some(value) = lookup.lookup(name) else {
        return Err(ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path: TIMEOUT_PATH.to_owned(),
        });
    };
    Ok(betterlite_env::number::<f64>(name, &value)?)
}

fn yaml_error<M>(path: &str, message: M) -> ConfigError
where
    M: Display,
{
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {message}")),
    }
}
