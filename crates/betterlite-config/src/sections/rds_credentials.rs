use betterlite_db::rds_credentials::{RdsCredentialInputs, RdsTimeout};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

// the keys keep the aws_ names LiteLLM's own settings use
#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    #[serde(rename = "aws_access_key_id")]
    access_key_id: Option<SecretString>,
    #[serde(rename = "aws_secret_access_key")]
    secret_access_key: Option<SecretString>,
    #[serde(rename = "aws_region_name")]
    region_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    session_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    role_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    web_identity_token: Option<SecretString>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub credentials: RdsCredentialInputs,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = self.general_settings;
        let credentials = RdsCredentialInputs {
            access_key_id: resolve_field(
                configured
                    .access_key_id
                    .map(|secret| secret.expose_secret().to_owned()),
                "access_key_id",
                lookup,
                &[],
            )?
            .map(SecretString::from),
            secret_access_key: resolve_field(
                configured
                    .secret_access_key
                    .map(|secret| secret.expose_secret().to_owned()),
                "secret_access_key",
                lookup,
                &[],
            )?
            .map(SecretString::from),
            region_name: resolve_field(configured.region_name, "region_name", lookup, &[])?,
            session_name: resolve_field(configured.session_name, "session_name", lookup, &[])?,
            profile_name: resolve_field(configured.profile_name, "profile_name", lookup, &[])?,
            role_name: resolve_field(configured.role_name, "role_name", lookup, &["AWS_ROLE_ARN"])?,
            web_identity_token: resolve_field(
                configured
                    .web_identity_token
                    .map(|secret| secret.expose_secret().to_owned()),
                "web_identity_token",
                lookup,
                &["AWS_WEB_IDENTITY_TOKEN_FILE"],
            )?
            .map(SecretString::from),
            timeout: RdsTimeout::SdkDefaults,
        };
        Ok(Settings { credentials })
    }
}

// an unset os.environ/ reference is a load error, so the environment only fills absent keys
fn resolve_field<L>(
    configured: Option<String>,
    field: &str,
    lookup: &L,
    aliases: &[&str],
) -> Result<Option<String>, ConfigError>
where
    L: EnvironmentLookup,
{
    let path = format!("general_settings.aws_{field}");
    if let Some(text) = configured {
        return resolve_string(text, &path, lookup).map(Some);
    }
    Ok(first_environment(lookup, field, aliases))
}

fn first_environment<L>(lookup: &L, field: &str, aliases: &[&str]) -> Option<String>
where
    L: EnvironmentLookup,
{
    let variable = format!("AWS_{}", field.to_ascii_uppercase());
    if let Some(value) = lookup.lookup(&variable) {
        return Some(value);
    }
    for alias in aliases {
        if let Some(value) = lookup.lookup(alias) {
            return Some(value);
        }
    }
    None
}
