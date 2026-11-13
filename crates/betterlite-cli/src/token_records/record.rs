use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};

use super::secret::ExposedSecret;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliTokenRecord {
    #[serde(default)]
    pub base_url: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub key: Option<SecretString>,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user_email: String,
    #[serde(default)]
    pub user_role: String,
    #[serde(default = "default_auth_header_name")]
    pub auth_header_name: String,
    #[serde(
        default,
        serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
    )]
    pub jwt_token: SecretString,
    #[serde(default)]
    pub timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedSecret>>>::serialize"
    )]
    pub refresh_token: Option<SecretString>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

impl Default for CliTokenRecord {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            key: None,
            user_id: String::new(),
            user_email: String::new(),
            user_role: String::new(),
            auth_header_name: default_auth_header_name(),
            jwt_token: SecretString::default(),
            timestamp: 0.0,
            expires_at: None,
            refresh_token: None,
            client_id: None,
            token_endpoint: None,
            revocation_endpoint: None,
            resource: None,
            team_id: None,
        }
    }
}

fn default_auth_header_name() -> String {
    "Authorization".to_owned()
}

#[must_use]
pub fn holds_secret(record: &CliTokenRecord) -> bool {
    if record.key.is_some() || record.refresh_token.is_some() {
        return true;
    }
    !record.jwt_token.expose_secret().is_empty()
}

#[must_use]
pub fn without_secret(record: &CliTokenRecord) -> CliTokenRecord {
    let mut stripped = record.clone();
    stripped.key = None;
    stripped.jwt_token = SecretString::default();
    stripped.refresh_token = None;
    stripped
}
