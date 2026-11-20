use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const KEY: &str = "litellm_settings.sse_keepalive_ping_interval_seconds";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(rename = "sse_keepalive_ping_interval_seconds")]
    ping_interval: Option<RawValue>,
}

// a number is the interval and any other shape is a load error that names the field
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Number(RawNumber),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PingSetting {
    Seconds(f64),
    Text(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    // ast-grep-ignore: no-long-names
    pub sse_keepalive_ping_interval_seconds: Option<PingSetting>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let ping_interval = self
            .litellm_settings
            .ping_interval
            .map(|value| ping_setting(value, lookup))
            .transpose()?;
        Ok(Settings {
            // ast-grep-ignore: no-long-names
            sse_keepalive_ping_interval_seconds: ping_interval,
        })
    }
}

fn ping_setting<L>(value: RawValue, lookup: &L) -> Result<PingSetting, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        RawValue::Number(RawNumber::Integer(integer)) => Ok(PingSetting::Seconds(
            integer
                .to_string()
                .parse::<f64>()
                .map_err(|source| ConfigError::InvalidFloat {
                    path: KEY.to_owned(),
                    source,
                })?,
        )),
        RawValue::Number(RawNumber::Float(float)) => Ok(PingSetting::Seconds(float)),
        // coerce_keepalive_interval parses this text later, so nan, inf and unusable text reach it as written
        RawValue::Number(RawNumber::Text(text)) => {
            Ok(PingSetting::Text(resolve_string(text, KEY, lookup)?))
        }
        RawValue::Number(RawNumber::Boolean(enabled)) => Err(ConfigError::InvalidScalar {
            path: KEY.to_owned(),
            value: enabled,
        }),
        RawValue::Other(_) => Err(ConfigError::InvalidScalar {
            path: KEY.to_owned(),
            value: false,
        }),
    }
}
