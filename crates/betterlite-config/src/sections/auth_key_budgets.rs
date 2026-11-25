mod tests;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

// constants.py: EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE = float(os.getenv(name, 0.8))
const ALERT_FRACTION_VARIABLE: &str = "EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE";
const DEFAULT_ALERT_FRACTION: f64 = 0.8;

// litellm.default_key_max_budget_alert_emails keeps whatever mapping the config declares; the auth checks normalize it
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub default_alert_emails: Option<BTreeMap<String, JsonData>>,
    pub alert_fraction: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_alert_emails: None,
            alert_fraction: DEFAULT_ALERT_FRACTION,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(rename = "default_key_max_budget_alert_emails", default)]
    default_alert_emails: Option<BTreeMap<String, JsonData>>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            default_alert_emails: self.litellm_settings.default_alert_emails,
            alert_fraction: alert_fraction(lookup)?,
        })
    }
}

// The constant is built at import, so text float() cannot read stops the proxy instead of falling back.
fn alert_fraction<L>(lookup: &L) -> Result<f64, ConfigError>
where
    L: EnvironmentLookup,
{
    let Some(text) = lookup.lookup(ALERT_FRACTION_VARIABLE) else {
        return Ok(DEFAULT_ALERT_FRACTION);
    };
    text.trim()
        .parse::<f64>()
        .map_err(|source| ConfigError::InvalidFloat {
            path: ALERT_FRACTION_VARIABLE.to_owned(),
            source,
        })
}
