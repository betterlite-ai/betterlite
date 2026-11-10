use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";

// a map-shaped Raw, not a unit struct, so serde still accepts (and ignores) the document's keys
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub azure_authority_host: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            azure_authority_host: DEFAULT_AUTHORITY_HOST.to_owned(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        // os.getenv keeps an empty value, so only an unset variable takes the default
        Ok(Settings {
            azure_authority_host: lookup
                .lookup("AZURE_AUTHORITY_HOST")
                .unwrap_or_else(|| DEFAULT_AUTHORITY_HOST.to_owned()),
        })
    }
}
