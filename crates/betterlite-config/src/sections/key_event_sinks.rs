use betterlite_secret_managers::secret_contracts::options::SecretTags;
use betterlite_secret_managers::secret_types::KeyManagementSystem;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::key_management_settings::{self, RawSettings};

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    alerting: Option<Vec<String>>,
    key_management_settings: Option<RawSettings>,
    key_management_system: Option<KeyManagementSystem>,
}

// the shared Sections snapshot requires Default; the real defaults come from B4410's conversion
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub store_virtual_keys: bool,
    pub prefix_for_stored_virtual_keys: String,
    pub description: Option<String>,
    pub tags: Option<SecretTags>,
    // general_settings.alerting only; no top-level alerting key switches the legacy email on
    pub alerting: Vec<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let key_management = key_management_settings::Raw {
            general_settings: key_management_settings::RawGeneral {
                key_management_settings: general.key_management_settings,
                key_management_system: general.key_management_system,
            },
        }
        .convert(lookup)?;
        Ok(Settings {
            store_virtual_keys: key_management.store_virtual_keys,
            prefix_for_stored_virtual_keys: key_management.prefix_for_stored_virtual_keys,
            description: key_management.description,
            tags: key_management.tags.map(SecretTags::Mapping),
            alerting: general.alerting.unwrap_or_default(),
        })
    }
}
