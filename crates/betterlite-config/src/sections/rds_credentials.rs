use betterlite_db::rds_credentials::{RdsCredentialInputs, RdsTimeout};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
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
            access_key_id: resolve_reference(configured.access_key_id, lookup)
                .or_else(|| lookup.lookup("AWS_ACCESS_KEY_ID").map(SecretString::from)),
            secret_access_key: resolve_reference(configured.secret_access_key, lookup).or_else(
                || {
                    lookup
                        .lookup("AWS_SECRET_ACCESS_KEY")
                        .map(SecretString::from)
                },
            ),
            region_name: resolve_text(configured.region_name, lookup)
                .or_else(|| lookup.lookup("AWS_REGION_NAME")),
            session_name: resolve_text(configured.session_name, lookup)
                .or_else(|| lookup.lookup("AWS_SESSION_NAME")),
            profile_name: resolve_text(configured.profile_name, lookup)
                .or_else(|| lookup.lookup("AWS_PROFILE_NAME")),
            role_name: resolve_text(configured.role_name, lookup)
                .or_else(|| lookup.lookup("AWS_ROLE_NAME"))
                .or_else(|| lookup.lookup("AWS_ROLE_ARN")),
            web_identity_token: resolve_reference(configured.web_identity_token, lookup)
                .or_else(|| {
                    lookup
                        .lookup("AWS_WEB_IDENTITY_TOKEN")
                        .map(SecretString::from)
                })
                .or_else(|| {
                    lookup
                        .lookup("AWS_WEB_IDENTITY_TOKEN_FILE")
                        .map(SecretString::from)
                }),
            timeout: RdsTimeout::SdkDefaults,
        };
        Ok(Settings { credentials })
    }
}

// get_secret resolves an os.environ/ reference to nothing when the variable is unset
fn resolve_reference<L>(value: Option<SecretString>, lookup: &L) -> Option<SecretString>
where
    L: EnvironmentLookup,
{
    let text = value?;
    let Some(name) = text.expose_secret().strip_prefix("os.environ/") else {
        return Some(text);
    };
    lookup.lookup(name).map(SecretString::from)
}

fn resolve_text<L>(value: Option<String>, lookup: &L) -> Option<String>
where
    L: EnvironmentLookup,
{
    let text = value?;
    let Some(name) = text.strip_prefix("os.environ/") else {
        return Some(text);
    };
    lookup.lookup(name)
}
