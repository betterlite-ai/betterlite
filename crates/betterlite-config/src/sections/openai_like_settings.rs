use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::{ExposeSecret as _, SecretString};

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
    // an explicit empty base or key falls through to the environment, and the key ends as an empty string because vllm needs none
    #[must_use]
    pub fn resolve(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> (Option<String>, SecretString) {
        let base = api_base
            .filter(|explicit| !explicit.is_empty())
            .map(str::to_owned)
            .or_else(|| self.api_base.clone());
        let key = api_key
            .filter(|explicit| !explicit.expose_secret().is_empty())
            .or(self.api_key.as_ref())
            .map_or_else(|| SecretString::from(String::new()), SecretString::clone);
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
                .lookup("OPENAI_LIKE_API_BASE")
                .filter(|value| !value.is_empty()),
            api_key: lookup
                .lookup("OPENAI_LIKE_API_KEY")
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
        })
    }
}
