use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::Expiry;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

// serializes the plain token so a record can leave the process, while the secret's Debug stays redacted
#[derive(Serialize)]
struct ExposedToken(String);

impl From<&SecretString> for ExposedToken {
    fn from(secret: &SecretString) -> Self {
        Self(secret.expose_secret().to_owned())
    }
}

#[derive(Debug, Serialize)]
pub struct SessionRecord {
    #[serde(
        serialize_with = "serde_with::As::<Option<serde_with::FromIntoRef<ExposedToken>>>::serialize"
    )]
    pub token: Option<SecretString>,
    pub key_name: Option<String>,
    pub key_alias: Option<String>,
    #[serde(serialize_with = "rust_decimal::serde::float_option::serialize")]
    pub max_budget: Option<Decimal>,
    pub expires: Option<String>,
    pub models: Vec<JsonData>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub team_models: Vec<JsonData>,
    pub team_model_aliases: Option<BTreeMap<String, JsonData>>,
    pub rpm_limit: Option<i64>,
    pub user_role: Option<UserRole>,
    pub is_session_token: bool,
    pub max_parallel_requests: Option<i64>,
}

impl SessionRecord {
    pub fn from_auth(auth: &UserAPIKeyAuth) -> Self {
        Self {
            token: auth.view.token.token.clone(),
            key_name: auth.view.token.key_name.clone(),
            key_alias: auth.view.token.key_alias.clone(),
            max_budget: auth.view.token.max_budget,
            expires: auth.view.token.expires.as_ref().map(expiry_text),
            models: auth.view.token.models.clone(),
            user_id: auth.view.token.user_id.clone(),
            team_id: auth.view.token.team_id.clone(),
            team_alias: auth.view.team_alias.clone(),
            team_models: auth.view.team_models.clone(),
            team_model_aliases: auth
                .view
                .team_model_aliases
                .clone()
                .map(|aliases| aliases.0),
            rpm_limit: auth.view.token.rpm_limit,
            user_role: auth.extra.user_role,
            is_session_token: auth.extra.is_session_token,
            max_parallel_requests: auth.view.token.max_parallel_requests,
        }
    }
}

fn expiry_text(expiry: &Expiry) -> String {
    match expiry {
        Expiry::Text(text) => text.clone(),
        Expiry::DateTime(time) => time.to_rfc3339(),
    }
}
