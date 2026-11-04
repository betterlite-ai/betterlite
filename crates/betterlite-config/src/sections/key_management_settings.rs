use std::collections::BTreeMap;

use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use serde::{Deserialize, de::IgnoredAny};

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod resolution;
mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawGeneral {
    pub key_management_settings: Option<RawSettings>,
    pub key_management_system: Option<KeyManagementSystem>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Boolean {
    Literal(bool),
    Environment(String),
}

#[derive(Debug, Default, Deserialize)]
pub struct RawSettings {
    pub hosted_keys: Option<Vec<String>>,
    pub store_virtual_keys: Option<Boolean>,
    pub prefix_for_stored_virtual_keys: Option<String>,
    pub access_mode: Option<String>,
    pub primary_secret_name: Option<String>,
    pub description: Option<String>,
    pub tags: Option<BTreeMap<String, String>>,
    pub kms_key_id: Option<String>,
    pub custom_secret_manager: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub replica_regions: Option<Vec<String>>,
    // This section deliberately follows Pydantic's ignore-extra contract.
    #[serde(flatten)]
    pub ignored: IgnoredAny,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub hosted_keys: Option<Vec<String>>,
    pub store_virtual_keys: bool,
    pub prefix_for_stored_virtual_keys: String,
    pub access_mode: SecretAccessMode,
    pub primary_secret_name: Option<String>,
    pub description: Option<String>,
    pub tags: Option<BTreeMap<String, String>>,
    pub kms_key_id: Option<String>,
    pub custom_secret_manager: Option<String>,
    pub aws_region_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
    pub aws_external_id: Option<String>,
    pub aws_profile_name: Option<String>,
    pub aws_web_identity_token: Option<SecretString>,
    pub aws_sts_endpoint: Option<String>,
    pub replica_regions: Option<Vec<String>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hosted_keys: None,
            store_virtual_keys: false,
            prefix_for_stored_virtual_keys: "litellm/".to_owned(),
            access_mode: SecretAccessMode::ReadOnly,
            primary_secret_name: None,
            description: None,
            tags: None,
            kms_key_id: None,
            custom_secret_manager: None,
            aws_region_name: None,
            aws_role_name: None,
            aws_session_name: None,
            aws_external_id: None,
            aws_profile_name: None,
            aws_web_identity_token: None,
            aws_sts_endpoint: None,
            replica_regions: None,
        }
    }
}

macro_rules! resolve_fields {
    ($settings:ident, $raw:ident, $lookup:ident; $($field:ident),+ $(,)?) => {
        $(
            $settings.$field = resolution::text(
                $raw.$field, concat!("general_settings.key_management_settings.", stringify!($field)), $lookup,
            )?;
        )+
    };
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self
            .general_settings
            .key_management_settings
            .unwrap_or_default();
        let mut settings = Settings::default();
        resolve_fields!(settings, raw, lookup;
            primary_secret_name, description, kms_key_id, custom_secret_manager,
            aws_region_name, aws_role_name, aws_session_name, aws_external_id,
            aws_profile_name, aws_sts_endpoint,
        );
        settings.hosted_keys = resolution::names(raw.hosted_keys, "hosted_keys", lookup)?;
        settings.replica_regions =
            resolution::names(raw.replica_regions, "replica_regions", lookup)?;
        settings.tags = resolution::tags(raw.tags, lookup)?;
        settings.store_virtual_keys = resolution::boolean(raw.store_virtual_keys, lookup)?;
        settings.access_mode = resolution::access(raw.access_mode, lookup)?;
        if let Some(prefix) = resolution::text(
            raw.prefix_for_stored_virtual_keys,
            "general_settings.key_management_settings.prefix_for_stored_virtual_keys",
            lookup,
        )? {
            settings.prefix_for_stored_virtual_keys = prefix;
        }
        settings.aws_web_identity_token = resolution::token(raw.aws_web_identity_token, lookup)?;
        if self.general_settings.key_management_system == Some(KeyManagementSystem::Custom)
            && settings
                .custom_secret_manager
                .as_ref()
                .is_none_or(|path| path.trim().is_empty())
        {
            return Err(ConfigError::InvalidScalar {
                path: "general_settings.key_management_settings.custom_secret_manager".to_owned(),
                value: false,
            });
        }
        Ok(settings)
    }
}
