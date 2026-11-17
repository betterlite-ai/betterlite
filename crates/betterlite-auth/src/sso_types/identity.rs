use std::collections::BTreeMap;

use betterlite_db::managed_resource_rows::wire::RowFlag;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use crate::jwt_claims::TypedValue;
use crate::sso_types::exposed_secret::ExposedSecret;
use crate::user_roles::UserRole;

// The OAuth bearer credentials LiteLLM keeps out of SSO debug responses, while it keeps token_type, expires_in and scope.
pub const OAUTH_TOKEN_FIELDS: [&str; 3] = ["access_token", "id_token", "refresh_token"];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenIdIdentity {
    pub id: Option<String>,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub display_name: Option<String>,
    pub picture: Option<String>,
    pub provider: Option<String>,
    pub team_ids: Vec<String>,
    pub user_role: Option<UserRole>,
    // a claim the provider sent beyond the OpenID shape; Python's _as_object cast adds no step here
    pub extra_fields: Option<BTreeMap<String, TypedValue>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    Sso,
    UsernamePassword,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReturnedUiTokenObject {
    pub user_id: String,
    #[serde(
        serialize_with = "serde_with::As::<serde_with::FromIntoRef<ExposedSecret>>::serialize"
    )]
    pub key: SecretString,
    pub user_email: Option<String>,
    pub user_role: String,
    pub login_method: LoginMethod,
    pub premium_user: RowFlag,
    pub auth_header_name: String,
    #[serde(rename = "disabled_non_admin_personal_key_creation")]
    pub personal_key_creation_disabled: RowFlag,
    pub server_root_path: String,
    pub password_reset_required: RowFlag,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ParsedOpenIdResult {
    pub user_email: Option<String>,
    pub user_id: Option<String>,
    pub user_role: Option<String>,
}
