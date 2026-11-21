use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const MISSING_SESSION_ID_PATH: &str = "general_settings.missing_session_id";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    missing_session_id: Option<RawScalar>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawScalar {
    Text(String),
    Other(IgnoredAny),
}

// LiteLLM compares the setting against its own literals and only warns about any other value at request time
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingSessionIdPolicy {
    Generate,
    Reject,
    Omit,
    Unrecognized(String),
}

impl MissingSessionIdPolicy {
    fn from_text(text: &str) -> Self {
        match text {
            "generate" => Self::Generate,
            "reject" => Self::Reject,
            "omit" => Self::Omit,
            other => Self::Unrecognized(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub missing_session_id: Option<MissingSessionIdPolicy>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let missing_session_id = match self.general_settings.missing_session_id {
            None => None,
            Some(RawScalar::Text(text)) => Some(MissingSessionIdPolicy::from_text(&text)),
            Some(RawScalar::Other(_)) => {
                return Err(ConfigError::InvalidScalar {
                    path: MISSING_SESSION_ID_PATH.to_owned(),
                    value: false,
                });
            }
        };
        Ok(Settings { missing_session_id })
    }
}
