use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const FAIL_CLOSED_SETTING: &str = "fail_closed_rate_limit_enforcement";
const WINDOW_SIZE_VARIABLE: &str = "LITELLM_RATE_LIMIT_WINDOW_SIZE";
const TPM_RESERVATION_VARIABLE: &str = "LITELLM_TPM_TOKEN_RESERVATION_ENABLED";
const WINDOW_SIZE_DEFAULT: i64 = 60;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    #[serde(rename = "fail_closed_rate_limit_enforcement", default)]
    fail_closed: RawFlag,
}

// the declared type is bool | None, so a null is the same as an absent key and every other shape warns
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Bool(bool),
    Text(String),
    Integer(i64),
    Float(f64),
    Null(()),
    Other(IgnoredAny),
    #[default]
    #[serde(skip_deserializing)]
    Absent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    pub fail_closed: bool,
    pub window_size: i64,
    pub tpm_reservation_enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fail_closed: false,
            window_size: WINDOW_SIZE_DEFAULT,
            tpm_reservation_enabled: true,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            fail_closed: read_fail_closed(self.general_settings.fail_closed, lookup),
            window_size: window_size(lookup)?,
            tpm_reservation_enabled: lookup
                .lookup(TPM_RESERVATION_VARIABLE)
                .is_none_or(|text| text.to_lowercase() == "true"),
        })
    }
}

fn read_fail_closed<L>(raw: RawFlag, lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    match raw {
        RawFlag::Bool(value) => value,
        RawFlag::Absent | RawFlag::Null(()) | RawFlag::Integer(0) | RawFlag::Float(0.0) => false,
        RawFlag::Text(text) => text_flag(&text, lookup),
        // pydantic's lax bool reads the integers 1 and 0 and the floats 1.0 and 0.0
        RawFlag::Integer(1) | RawFlag::Float(1.0) => true,
        other @ (RawFlag::Integer(_) | RawFlag::Float(_) | RawFlag::Other(_)) => {
            warn_not_boolean(&rendered(&other));
            false
        }
    }
}

fn text_flag<L>(text: &str, lookup: &L) -> bool
where
    L: EnvironmentLookup,
{
    let resolved = text.strip_prefix("os.environ/").map_or_else(
        || text.to_owned(),
        |variable| lookup.lookup(variable).unwrap_or_default(),
    );
    match betterlite_env::flag(FAIL_CLOSED_SETTING, &resolved) {
        Ok(Some(value)) => value,
        Ok(None) => false,
        Err(_) => {
            warn_not_boolean(&resolved);
            false
        }
    }
}

fn warn_not_boolean(value: &str) {
    tracing::warn!(
        "general_settings.{}={} is not a boolean, treating it as disabled",
        FAIL_CLOSED_SETTING,
        value
    );
}

fn rendered(raw: &RawFlag) -> String {
    match raw {
        RawFlag::Integer(number) => number.to_string(),
        RawFlag::Float(number) => number.to_string(),
        RawFlag::Other(_) => "a list or a map".to_owned(),
        RawFlag::Bool(_) | RawFlag::Text(_) | RawFlag::Null(()) | RawFlag::Absent => String::new(),
    }
}

fn window_size<L>(lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(WINDOW_SIZE_VARIABLE) else {
        return Ok(WINDOW_SIZE_DEFAULT);
    };
    let seconds =
        betterlite_env::number::<i64>(WINDOW_SIZE_VARIABLE, &text)?.unwrap_or(WINDOW_SIZE_DEFAULT);
    if seconds <= 0 {
        return Err(ConfigError::from(betterlite_env::EnvError::Number {
            name: WINDOW_SIZE_VARIABLE.to_owned(),
        }));
    }
    Ok(seconds)
}
