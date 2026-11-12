use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use secrecy::SecretString;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

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
        // the embedding adapter reads only COMETAPI_KEY; the image adapter's COMETAPI_API_KEY alias stays out
        let api_key = lookup
            .lookup("COMETAPI_KEY")
            .filter(|value| !value.is_empty())
            .map(SecretString::from);
        Ok(Settings { api_key })
    }
}
