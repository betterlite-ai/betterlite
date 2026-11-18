use serde::Deserialize;

use crate::credentials::CredentialItem;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    credential_list: Option<Vec<CredentialItem>>,
}

#[derive(Debug, Clone, Default)]
pub struct CredentialListSettings {
    pub credentials: Vec<CredentialItem>,
}

// boxed so a new section adds one pointer to Sections, not 24 inline bytes that reached clippy's 16384-byte large_futures threshold in a db-admin future
pub type Settings = Box<CredentialListSettings>;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Box::new(CredentialListSettings {
            credentials: self.credential_list.unwrap_or_default(),
        }))
    }
}
