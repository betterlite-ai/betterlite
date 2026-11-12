use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::bearer::encrypt_bearer_token;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde::Serialize;

use crate::session_token_codec::LITELLM_SESSION_TOKEN_PREFIX;
use crate::session_token_codec::builders::SessionTokenError;
use crate::user_roles::UserRole;

// field order matches pydantic's declaration order so the encrypted bytes match the pin's
#[derive(Serialize)]
pub struct SessionPayload {
    pub token: String,
    pub key_name: String,
    pub key_alias: String,
    #[serde(serialize_with = "rust_decimal::serde::float::serialize")]
    pub spend: Decimal,
    #[serde(serialize_with = "rust_decimal::serde::float::serialize")]
    pub total_spend: Decimal,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "rust_decimal::serde::float_option::serialize"
    )]
    pub max_budget: Option<Decimal>,
    pub expires: String,
    pub models: Vec<JsonData>,
    pub aliases: Payload,
    pub config: Payload,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    pub metadata: Payload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<i64>,
    #[serde(flatten)]
    pub defaults: SessionDefaults,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_alias: Option<String>,
    pub team_models: Vec<String>,
    pub team_blocked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_model_aliases: Option<BTreeMap<String, JsonData>>,
    pub user_role: UserRole,
    pub is_session_token: bool,
}

#[derive(Serialize)]
pub struct SessionDefaults {
    allowed_cache_controls: Vec<JsonData>,
    allowed_routes: Vec<JsonData>,
    permissions: Payload,
    model_spend: Payload,
    model_max_budget: Payload,
    budget_fallbacks: BTreeMap<String, Vec<String>>,
    soft_budget_cooldown: bool,
    rotation_count: i64,
    auto_rotate: bool,
}

pub const SESSION_DEFAULTS: SessionDefaults = SessionDefaults {
    allowed_cache_controls: Vec::new(),
    allowed_routes: Vec::new(),
    permissions: Payload(BTreeMap::new()),
    model_spend: Payload(BTreeMap::new()),
    model_max_budget: Payload(BTreeMap::new()),
    budget_fallbacks: BTreeMap::new(),
    soft_budget_cooldown: false,
    rotation_count: 0,
    auto_rotate: false,
};

// the fields that differ between the UI and CLI records; the shared envelope lives in seal_session
pub struct SessionShape {
    pub token: String,
    pub key_name: String,
    pub key_alias: String,
    pub max_budget: Option<Decimal>,
    pub expires: String,
    pub models: Vec<JsonData>,
    pub user_id: String,
    pub team_id: Option<String>,
    pub rpm_limit: Option<i64>,
    pub team_alias: Option<String>,
    pub team_models: Vec<String>,
    pub team_model_aliases: Option<BTreeMap<String, JsonData>>,
    pub user_role: UserRole,
    pub is_session_token: bool,
}

pub fn seal_session<N, D>(
    cipher: &StoredCipher<N, D>,
    shape: SessionShape,
) -> Result<SecretString, SessionTokenError>
where
    N: NonceSource,
{
    let payload = SessionPayload {
        token: shape.token,
        key_name: shape.key_name,
        key_alias: shape.key_alias,
        spend: Decimal::ZERO,
        total_spend: Decimal::ZERO,
        max_budget: shape.max_budget,
        expires: shape.expires,
        models: shape.models,
        aliases: Payload::default(),
        config: Payload::default(),
        user_id: shape.user_id,
        team_id: shape.team_id,
        metadata: Payload::default(),
        rpm_limit: shape.rpm_limit,
        defaults: SESSION_DEFAULTS,
        team_alias: shape.team_alias,
        team_models: shape.team_models,
        team_blocked: false,
        team_model_aliases: shape.team_model_aliases,
        user_role: shape.user_role,
        is_session_token: shape.is_session_token,
    };
    let json =
        serde_json::to_string(&payload).map_err(|source| SessionTokenError::Encode { source })?;
    encrypt_bearer_token(
        cipher,
        &SecretString::from(json),
        LITELLM_SESSION_TOKEN_PREFIX,
    )
    .map(SecretString::from)
    .map_err(seal_error)
}

fn seal_error(source: SecretError) -> SessionTokenError {
    match source {
        SecretError::BearerSecret => SessionTokenError::MissingSecret,
        other @ (SecretError::MissingKey
        | SecretError::TooLarge { .. }
        | SecretError::Base64
        | SecretError::Decode { .. }
        | SecretError::Truncated
        | SecretError::Seal { .. }
        | SecretError::LegacySeal { .. }
        | SecretError::Utf8 { .. }
        | SecretError::Random { .. }
        | SecretError::MapEncode { .. }
        | SecretError::MapDecode { .. }) => SessionTokenError::Encrypt { source: other },
    }
}
