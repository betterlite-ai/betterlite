use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// Azure Identity and MSAL read these three names from the process environment; LiteLLM never forwards them as config
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub azure_regional_authority_name: Option<String>,
    pub msal_force_region: Option<String>,
    pub region_name: String,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            azure_regional_authority_name: lookup.lookup("AZURE_REGIONAL_AUTHORITY_NAME"),
            msal_force_region: lookup.lookup("MSAL_FORCE_REGION"),
            region_name: lookup.lookup("REGION_NAME").unwrap_or_default(),
        })
    }
}

mod tests;
