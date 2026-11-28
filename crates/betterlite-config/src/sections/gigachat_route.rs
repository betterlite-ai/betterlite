use betterlite_secret_managers::secret_lookup::text_to_bool;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const API_BASE_VARIABLE: &str = "GIGACHAT_API_BASE";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    // A skipped marker keeps this environment-only section a YAML mapping.
    #[serde(skip)]
    _environment: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|value| !value.is_empty() && text_to_bool(value).is_none()),
        })
    }
}

mod tests;
