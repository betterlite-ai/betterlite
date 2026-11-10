use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub clarifai_api_key: Option<SecretString>,
    pub compactifai_api_key: Option<SecretString>,
    pub datarobot_api_token: Option<SecretString>,
    pub datarobot_endpoint: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let key = |name: &str| {
            lookup
                .lookup(name)
                .filter(|value| !value.is_empty())
                .map(SecretString::from)
        };
        Ok(Settings {
            clarifai_api_key: key("CLARIFAI_API_KEY"),
            compactifai_api_key: key("COMPACTIFAI_API_KEY"),
            datarobot_api_token: key("DATAROBOT_API_TOKEN"),
            // LiteLLM tests the endpoint with `is None` after its fallback, so a set-but-empty endpoint still reaches the path edit
            datarobot_endpoint: lookup.lookup("DATAROBOT_ENDPOINT"),
        })
    }
}
