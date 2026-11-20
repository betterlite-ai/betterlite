#[path = "team_fields/tests.rs"]
mod tests;

pub mod schema;

use std::fmt::Display;
use std::time::Duration;

use serde::Deserialize;
use serde::de::{Error as DeserializeError, IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

use self::schema::{MetadataSchemaField, parse_team_metadata_schema};

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
            validator: resolve_validator(general.validator, lookup)?,
            schema: parse_team_metadata_schema(general.schema.as_ref())
                .map_err(|source| yaml_error(SCHEMA_PATH, source))?,
            timeout: resolve_timeout(general.timeout, lookup)?,
            unavailable_message: resolve_message(general.message, lookup)?,
        })
    }
}

// get_instance_fn reads the name off a str, so any other JSON shape fails the config load instead of validation.
fn resolve_validator<L>(raw: Option<RawScalar>, lookup: &L) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(None);
    };
    let RawScalar::Text(text) = raw else {
        return Err(yaml_error(
            VALIDATOR_PATH,
            "the value must be a string naming a callable",
        ));
    };
    let name = resolve_string(text, VALIDATOR_PATH, lookup)?;
    if name.is_empty() {
        return Err(yaml_error(
            VALIDATOR_PATH,
            "the value must be a string naming a callable",
        ));
    }
    Ok(Some(name))
}

// A positive number is a deadline, positive infinity has none, and every other scalar keeps the default.
fn resolve_timeout<L>(raw: Option<RawScalar>, lookup: &L) -> Result<ValidationTimeout, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT));
    };
    let seconds = match raw {
        RawScalar::Integer(seconds) => return Ok(integer_timeout(seconds)),
        RawScalar::Float(seconds) => seconds,
        RawScalar::Text(text) => match environment_seconds(&text, lookup)? {
            Some(seconds) => seconds,
            None => return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT)),
        },
        RawScalar::Boolean(_) | RawScalar::Other(_) => {
            return Ok(ValidationTimeout::Finite(DEFAULT_TIMEOUT));
        }
    };
    Ok(float_timeout(seconds))
}

fn integer_timeout(seconds: i64) -> ValidationTimeout {
    match u64::try_from(seconds) {
        Ok(0) | Err(_) => ValidationTimeout::Finite(DEFAULT_TIMEOUT),
        Ok(value) => ValidationTimeout::Finite(Duration::from_secs(value)),
    }
}

fn float_timeout(seconds: f64) -> ValidationTimeout {
    if seconds.is_nan() || seconds <= 0.0 {
        return ValidationTimeout::Finite(DEFAULT_TIMEOUT);
    }
    if seconds.is_infinite() {
        return ValidationTimeout::Unlimited;
    }
    match Duration::try_from_secs_f64(seconds) {
        Ok(duration) => ValidationTimeout::Finite(duration),
        Err(_) => ValidationTimeout::Unlimited,
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

// A nonblank string is used as written, spaces included; empty, blank and non-string values keep the default.
fn resolve_message<L>(raw: Option<RawScalar>, lookup: &L) -> Result<String, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(raw) = raw else {
        return Ok(DEFAULT_UNAVAILABLE_MESSAGE.to_owned());
    };
    let RawScalar::Text(text) = raw else {
        return Ok(DEFAULT_UNAVAILABLE_MESSAGE.to_owned());
    };
    let message = resolve_string(text, MESSAGE_PATH, lookup)?;
    if message.trim().is_empty() {
        return Ok(DEFAULT_UNAVAILABLE_MESSAGE.to_owned());
    }
    Ok(message)
}

fn yaml_error<M>(path: &str, message: M) -> ConfigError
where
    M: Display,
{
    ConfigError::Yaml {
        source: <serde_saphyr::Error as DeserializeError>::custom(format!("{path}: {message}")),
    }
}
