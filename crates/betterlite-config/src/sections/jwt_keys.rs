use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub const PUBLIC_KEY_URL_VARIABLE: &str = "JWT_PUBLIC_KEY_URL";

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    // None means the variable is absent; Some("") is how an operator unset it in the environment
    pub public_key_url: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            public_key_url: lookup.lookup(PUBLIC_KEY_URL_VARIABLE),
        })
    }
}

mod tests;
