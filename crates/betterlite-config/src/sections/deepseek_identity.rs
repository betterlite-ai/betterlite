use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::{ExposeSecret as _, SecretString};

pub const DEFAULT_API_BASE: &str = "https://api.deepseek.com/beta";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

impl Settings {
    // The pinned resolver reads explicit values with Python's `or`, so an empty one falls through
    #[must_use]
    pub fn resolve(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (String, Option<SecretString>) {
        let base = api_base
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| self.api_base.clone())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        let key = api_key
            .filter(|value| !value.expose_secret().is_empty())
            .cloned()
            .or_else(|| self.api_key.clone());
        (base, key)
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            api_base: lookup
                .lookup("DEEPSEEK_API_BASE")
                .filter(|value| !value.is_empty()),
            api_key: lookup
                .lookup("DEEPSEEK_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
