mod tests;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const OUTPUT_PARSE_PII_PATH: &str = "litellm_settings.output_parse_pii";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default, rename = "litellm_settings")]
    settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    output_parse_pii: Option<RawFlag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawFlag {
    Boolean(bool),
    Other(IgnoredAny),
}

// LiteLLM's module-level litellm.output_parse_pii, set from litellm_settings
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub output_parse_pii: bool,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            output_parse_pii: flag(self.settings.output_parse_pii.as_ref())?,
        })
    }
}

fn flag(raw: Option<&RawFlag>) -> Result<bool, ConfigError> {
    match raw {
        None => Ok(false),
        Some(RawFlag::Boolean(value)) => Ok(*value),
        Some(RawFlag::Other(_)) => Err(ConfigError::InvalidScalar {
            path: OUTPUT_PARSE_PII_PATH.to_owned(),
            value: false,
        }),
    }
}
