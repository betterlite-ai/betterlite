use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde::de::Error as DeserializeError;

use crate::credentials::{CredentialItem, parse_credential_list};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    credential_list: Option<JsonData>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub credentials: Vec<CredentialItem>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let credentials =
            parse_credential_list(self.credential_list).map_err(|error| ConfigError::Yaml {
                source: <serde_saphyr::Error as DeserializeError>::custom(error.to_string()),
            })?;
        Ok(Settings { credentials })
    }
}
