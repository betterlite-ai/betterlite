use betterlite_prompts::arguments::json::Object;
use reqwest::StatusCode;
use secrecy::SecretString;
use thiserror::Error;

pub mod apply_claim_operations;
pub mod build_channel_token_claims;
pub mod build_claims;
pub mod build_debug_header;
pub mod build_scope;
pub mod passthrough_optional_claims;
pub mod resolve_end_user_identity;
pub mod validate_required_claims;

pub type Claims = Object;

#[derive(Debug)]
pub struct AuthFields {
    pub user_id: Option<String>,
    pub user_email: Option<String>,
    pub end_user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub token: Option<SecretString>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug)]
pub struct ClaimSettings {
    pub issuer: String,
    pub audience: String,
    pub ttl_seconds: i64,
    pub allowed_scopes: Option<Vec<String>>,
    pub add_claims: Object,
    pub set_claims: Object,
    pub remove_claims: Vec<String>,
    pub required_claims: Vec<String>,
    pub optional_claims: Vec<String>,
    pub end_user_claim_sources: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClaimError {
    #[error(
        "the incoming token is missing required claims: {missing:?}; configure the IdP to include these claims"
    )]
    MissingRequiredClaims { missing: Vec<String> },
    #[error("the configured token lifetime overflows the JWT timestamp range")]
    TimestampOverflow,
}

impl ClaimError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::MissingRequiredClaims { .. } => StatusCode::FORBIDDEN,
            Self::TimestampOverflow => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
