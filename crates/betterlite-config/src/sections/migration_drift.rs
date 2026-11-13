use secrecy::SecretString;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// Every input this component reads is an environment variable, so no YAML key belongs to this section.
#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(skip)]
    _unused: (),
}

// Boxed: Sections holds each Settings by value, and config snapshots live inside futures
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub inner: Box<Inner>,
}

#[derive(Debug, Clone, Default)]
pub struct Inner {
    pub database_url: Option<SecretString>,
    pub direct_url: Option<SecretString>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            inner: Box::new(Inner {
                database_url: lookup
                    .lookup("DATABASE_URL")
                    .filter(|text| !text.is_empty())
                    .map(SecretString::from),
                direct_url: lookup
                    .lookup("DIRECT_URL")
                    .filter(|text| !text.is_empty())
                    .map(SecretString::from),
            }),
        })
    }
}
