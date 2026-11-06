use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(rename = "budget_exceeded_throttle_percentage")]
    budget_exceeded_throttle_pct: Option<RawSetting>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RawSetting {
    Integer(i64),
    Float(f64),
    Text(String),
    Boolean(bool),
}

// the consumer validates the fraction, so an out-of-range or nonfinite value stays as configured
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub throttle_pct: Option<f64>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = "litellm_settings.budget_exceeded_throttle_percentage";
        let configured = match self.litellm_settings.budget_exceeded_throttle_pct {
            None | Some(RawSetting::Boolean(_)) => None,
            Some(RawSetting::Integer(value)) => value.to_f64(),
            Some(RawSetting::Float(value)) => Some(value),
            Some(RawSetting::Text(text)) => text_value(&text, path, lookup)?,
        };
        Ok(Settings {
            throttle_pct: configured,
        })
    }
}

fn text_value<L>(text: &str, path: &str, lookup: &L) -> Result<Option<f64>, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(name) = text.strip_prefix("os.environ/") else {
        let parsed = text
            .parse::<f64>()
            .map_err(|source| ConfigError::InvalidFloat {
                path: path.to_owned(),
                source,
            })?;
        return Ok(Some(parsed));
    };
    let Some(value) = lookup.lookup(name) else {
        return Err(ConfigError::MissingEnvironment {
            variable: name.to_owned(),
            path: path.to_owned(),
        });
    };
    Ok(betterlite_env::number::<f64>(name, &value)?)
}
