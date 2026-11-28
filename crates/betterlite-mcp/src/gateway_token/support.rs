pub mod reply;
mod tests;

use aws_lc_rs::error::Unspecified;

use crate::byok_token_exchange::pkce::verify_pkce;
use crate::gateway_authorize::oauth_error;
use crate::gateway_dcr::contracts::{DB_FAULTED_DESCRIPTION, DB_UNAVAILABLE_DESCRIPTION};
use crate::gateway_dcr::oauth_error::{OauthError, OauthErrorCode};
use crate::gateway_dcr::ports::{ProxyCredentialMintFailure, ReloadUserFailure};
use crate::gateway_dcr::subject_token::{RefusalCode, SubjectTokenRefusal};
use crate::session_credentials::SessionSigningConfigError;
use crate::types::SessionSigningKeys;

pub const BEARER_TOKEN_TYPE: &str = "Bearer";

pub trait SessionEntropy: Send + Sync {
    fn jti(&self) -> Result<[u8; 16], Unspecified>;
}

#[derive(Debug)]
pub enum SigningSetup {
    NoMasterKey,
    Invalid(SessionSigningConfigError),
    Ready(SessionSigningKeys),
}

pub fn signing_refusal(setup: &SigningSetup) -> Result<&SessionSigningKeys, OauthError> {
    match setup {
        SigningSetup::Ready(keys) => Ok(keys),
        SigningSetup::NoMasterKey => Err(no_master_key_error()),
        SigningSetup::Invalid(_error) => Err(signing_config_error()),
    }
}

#[must_use]
pub fn no_master_key_error() -> OauthError {
    oauth_error(
        500,
        OauthErrorCode::ServerError,
        "the gateway has no master key configured",
    )
}

#[must_use]
pub fn signing_config_error() -> OauthError {
    oauth_error(
        500,
        OauthErrorCode::ServerError,
        "the gateway session signing configuration is invalid",
    )
}

#[must_use]
pub fn failed_to_mint_error() -> OauthError {
    oauth_error(
        500,
        OauthErrorCode::ServerError,
        "failed to mint the session credential",
    )
}

#[must_use]
pub fn unknown_client_error() -> OauthError {
    oauth_error(
        401,
        OauthErrorCode::InvalidClient,
        "unknown or malformed client_id",
    )
}

#[must_use]
pub fn reload_failure_error(failure: ReloadUserFailure) -> OauthError {
    match failure {
        ReloadUserFailure::Unavailable => oauth_error(
            503,
            OauthErrorCode::TemporarilyUnavailable,
            DB_UNAVAILABLE_DESCRIPTION,
        ),
        ReloadUserFailure::Faulted => oauth_error(
            503,
            OauthErrorCode::TemporarilyUnavailable,
            DB_FAULTED_DESCRIPTION,
        ),
        ReloadUserFailure::Unresolvable => oauth_error(
            500,
            OauthErrorCode::ServerError,
            "the gateway is not configured to resolve users",
        ),
        ReloadUserFailure::NoActiveKey => oauth_error(
            400,
            OauthErrorCode::InvalidGrant,
            "the user for this grant is no longer active",
        ),
    }
}

#[must_use]
pub fn subject_refusal_error(refusal: SubjectTokenRefusal) -> OauthError {
    match refusal.error {
        RefusalCode::TemporarilyUnavailable => oauth_error(
            503,
            OauthErrorCode::TemporarilyUnavailable,
            &refusal.description,
        ),
        RefusalCode::UnsupportedGrantType => oauth_error(
            400,
            OauthErrorCode::UnsupportedGrantType,
            &refusal.description,
        ),
        RefusalCode::InvalidRequest => {
            oauth_error(400, OauthErrorCode::InvalidRequest, &refusal.description)
        }
    }
}

#[must_use]
pub fn mint_failure_error(failure: ProxyCredentialMintFailure) -> OauthError {
    match failure {
        ProxyCredentialMintFailure::NotAMember => oauth_error(
            400,
            OauthErrorCode::InvalidGrant,
            "the user is no longer a member of the team this grant was issued for",
        ),
        ProxyCredentialMintFailure::TeamRequired => oauth_error(
            400,
            OauthErrorCode::InvalidGrant,
            "this user belongs to a team; sign in again and pick the team for this credential",
        ),
        ProxyCredentialMintFailure::Unresolvable => {
            reload_failure_error(ReloadUserFailure::Unresolvable)
        }
        ProxyCredentialMintFailure::Unavailable => {
            reload_failure_error(ReloadUserFailure::Unavailable)
        }
        ProxyCredentialMintFailure::Faulted => reload_failure_error(ReloadUserFailure::Faulted),
        ProxyCredentialMintFailure::NoActiveKey => {
            reload_failure_error(ReloadUserFailure::NoActiveKey)
        }
    }
}

// Python's encode("ascii", "replace") swaps each non-ASCII code point for one "?", then hashes
#[must_use]
pub fn pkce_verifier_matches(code_verifier: &str, code_challenge: &str) -> bool {
    let ascii: String = code_verifier
        .chars()
        .map(|character| if character.is_ascii() { character } else { '?' })
        .collect();
    verify_pkce(&ascii, code_challenge)
}
