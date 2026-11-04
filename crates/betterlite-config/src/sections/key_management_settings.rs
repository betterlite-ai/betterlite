#[path = "key_management_settings/tests.rs"]
mod tests;

use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::mem;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    pub general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawGeneral {
    #[serde(default)]
    pub key_management_settings: RawSettings,
    pub key_management_system: Option<KeyManagementSystem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RawSettings {
    pub hosted_keys: Option<Vec<String>>,
    pub store_virtual_keys: Option<bool>,
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

impl Default for RawSettings {
    fn default() -> Self {
        Self {
            hosted_keys: None,
            store_virtual_keys: Some(false),
            prefix_for_stored_virtual_keys: String::from("litellm/"),
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

pub type Settings = RawSettings;

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let key_management_system = self.general_settings.key_management_system;
        let settings = resolve_settings(self.general_settings.key_management_settings, lookup)?;
        require_custom_manager(key_management_system, &settings)?;
        Ok(settings)
    }
}

macro_rules! resolve_setting_fields {
    ($settings:ident, $lookup:ident; $($field:ident, $path:literal),+ $(,)?) => {
        $(
            $settings.$field = resolve_setting_text(
                $settings.$field.take(),
                $path,
                $lookup,
            )?;
        )+
    };
}

fn resolve_settings<L>(mut settings: RawSettings, lookup: &L) -> Result<Settings, ConfigError>
where
    L: EnvironmentLookup,
{
    settings.custom_secret_manager = resolve_setting_text(
        settings.custom_secret_manager.take(),
        "key_management_settings.custom_secret_manager",
        lookup,
    )?;
    settings.prefix_for_stored_virtual_keys = resolve_string(
        mem::take(&mut settings.prefix_for_stored_virtual_keys),
        "key_management_settings.prefix_for_stored_virtual_keys",
        lookup,
    )?;
    resolve_setting_fields!(settings, lookup;
        primary_secret_name, "key_management_settings.primary_secret_name",
        description, "key_management_settings.description",
        kms_key_id, "key_management_settings.kms_key_id",
        aws_region_name, "key_management_settings.aws_region_name",
        aws_role_name, "key_management_settings.aws_role_name",
        aws_session_name, "key_management_settings.aws_session_name",
        aws_external_id, "key_management_settings.aws_external_id",
        aws_profile_name, "key_management_settings.aws_profile_name",
        aws_web_identity_token, "key_management_settings.aws_web_identity_token",
        aws_sts_endpoint, "key_management_settings.aws_sts_endpoint",
    );
    Ok(settings)
}

fn require_custom_manager(
    key_management_system: Option<KeyManagementSystem>,
    settings: &Settings,
) -> Result<(), ConfigError> {
    if key_management_system == Some(KeyManagementSystem::Custom)
        && settings.custom_secret_manager.is_none()
    {
        return Err(ConfigError::InvalidScalar {
            path: "key_management_settings.custom_secret_manager".to_owned(),
            value: false,
        });
    }
    Ok(())
}

trait SettingText: Sized {
    fn into_text(self) -> String;
    fn from_text(text: String) -> Self;
}

impl SettingText for String {
    fn into_text(self) -> String {
        self
    }

    fn from_text(text: String) -> Self {
        text
    }
}

impl SettingText for SecretString {
    fn into_text(self) -> String {
        secrecy::ExposeSecret::expose_secret(&self).to_owned()
    }

    fn from_text(text: String) -> Self {
        Self::from(text)
    }
}

fn resolve_setting_text<L, T>(
    value: Option<T>,
    path: &str,
    lookup: &L,
) -> Result<Option<T>, ConfigError>
where
    L: EnvironmentLookup,
    T: SettingText,
{
    value
        .map(|text| resolve_string(text.into_text(), path, lookup).map(T::from_text))
        .transpose()
}
