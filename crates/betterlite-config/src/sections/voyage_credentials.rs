use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub api_key: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let api_key = lookup
            .lookup("VOYAGE_API_KEY")
            .filter(|value| !value.is_empty())
            .or_else(|| {
                lookup
                    .lookup("VOYAGE_AI_API_KEY")
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| {
                lookup
                    .lookup("VOYAGE_AI_TOKEN")
                    .filter(|value| !value.is_empty())
            });
        Ok(Settings {
            api_key: api_key.map(SecretString::from),
        })
    }
}
