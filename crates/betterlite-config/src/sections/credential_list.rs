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
pub struct Settings {
    pub credentials: Vec<CredentialItem>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(Settings {
            credentials: self.credential_list.unwrap_or_default(),
        })
    }
}
