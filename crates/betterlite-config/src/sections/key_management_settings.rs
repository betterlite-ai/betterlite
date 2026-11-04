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

// The shared Sections snapshot requires Default; config defaults are resolved in convert.
#[derive(Debug, Clone, Default)]
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
    pub replica_regions: Vec<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let raw = general.key_management_settings.unwrap_or_default();
        let resolve = resolution::Resolver { lookup };
        Ok(Settings {
            hosted_keys: resolve.names(raw.hosted_keys, "hosted_keys")?,
            store_virtual_keys: raw
                .store_virtual_keys
                .unwrap_or(Boolean::Literal(false))
                .resolve(lookup)?,
            prefix_for_stored_virtual_keys: resolve
                .text(
                    raw.prefix_for_stored_virtual_keys,
                    "prefix_for_stored_virtual_keys",
                )?
                .unwrap_or_else(|| "litellm/".to_owned()),
            access_mode: resolution::access(
                resolve.text(raw.access_mode, "access_mode")?.as_deref(),
            )?,
            primary_secret_name: resolve.text(raw.primary_secret_name, "primary_secret_name")?,
            description: resolve.text(raw.description, "description")?,
            tags: resolve.tags(raw.tags)?,
            kms_key_id: resolve.text(raw.kms_key_id, "kms_key_id")?,
            custom_secret_manager: resolve
                .custom(raw.custom_secret_manager, general.key_management_system)?,
            aws_region_name: resolve.text(raw.aws_region_name, "aws_region_name")?,
            aws_role_name: resolve.text(raw.aws_role_name, "aws_role_name")?,
            aws_session_name: resolve.text(raw.aws_session_name, "aws_session_name")?,
            aws_external_id: resolve.text(raw.aws_external_id, "aws_external_id")?,
            aws_profile_name: resolve.text(raw.aws_profile_name, "aws_profile_name")?,
            aws_web_identity_token: resolve
                .text(raw.aws_web_identity_token, "aws_web_identity_token")?,
            aws_sts_endpoint: resolve.text(raw.aws_sts_endpoint, "aws_sts_endpoint")?,
            replica_regions: resolve
                .names(raw.replica_regions, "replica_regions")?
                .unwrap_or_default(),
        })
    }
}
