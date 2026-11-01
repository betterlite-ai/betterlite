use chrono::DateTime;
use serde::Deserialize;
use serde_json::json;

use betterlite_mcp::errors::{SessionKeyError, SessionMintError};
use betterlite_mcp::session_credentials::{self, SessionBearerResult, SessionRefreshResult};
use betterlite_mcp::session_tokens::mint_session_token;
use betterlite_mcp::types::{SessionKeys, SessionPrincipal, SessionSigningKeys, SessionTokenKind};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::helpers::{HelperError, HelperResult};

use super::bridge::TimestampOutOfRange;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionFields {
    signing_key: String,
    open_signing_key: Option<String>,
    candidate: Option<String>,
    mint: Option<SessionMint>,
    open_now: i64,
    #[serde(default)]
    expected_client_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionMint {
    principal: SessionPrincipal,
    kind: SessionTokenKind,
    mint_now: i64,
}

#[derive(Debug, Error)]
enum CredentialFailure {
    #[error(transparent)]
    Key(#[from] SessionKeyError),
    #[error(transparent)]
    Mint(#[from] SessionMintError),
}

impl From<CredentialFailure> for Box<HelperError> {
    fn from(failure: CredentialFailure) -> Self {
        let class = match &failure {
            CredentialFailure::Key(_) => "SessionKeyError",
            CredentialFailure::Mint(_) => "SessionMintError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

struct MissingExpectedClientId;

impl From<MissingExpectedClientId> for Box<HelperError> {
    fn from(_missing_expected_client_id: MissingExpectedClientId) -> Self {
        Self::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "expected_client_id is required".to_owned(),
        })
    }
}

fn candidate_and_open_keys(
    fields: &SessionFields,
) -> Result<(String, SessionSigningKeys), Box<HelperError>> {
    let mint_keys = SessionKeys::new(SecretString::from(fields.signing_key.as_str()))
        .map(SessionSigningKeys::Symmetric)
        .map_err(CredentialFailure::from)?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => {
            let now = DateTime::from_timestamp(mint.mint_now, 0).ok_or(TimestampOutOfRange)?;
            let minted = mint_session_token(&mint.principal, mint.kind, &mint_keys, now, || {
                Ok([0_u8; 16])
            })
            .map_err(CredentialFailure::from)?;
            minted.token.expose_secret().to_owned()
        }
        (None, Some(literal)) => literal.clone(),
        (None, None) => {
            return Err(Box::new(HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "either mint or candidate is required".to_owned(),
            }));
        }
    };
    let open_keys = match &fields.open_signing_key {
        Some(signing) => SessionKeys::new(SecretString::from(signing.as_str()))
            .map(SessionSigningKeys::Symmetric)
            .map_err(CredentialFailure::from)?,
        None => mint_keys,
    };
    Ok((candidate, open_keys))
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SessionOperation {
    Bearer,
    Refresh,
}

pub(crate) fn run_session(fields: &SessionFields, operation: SessionOperation) -> HelperResult {
    let (candidate, open_keys) = candidate_and_open_keys(fields)?;
    let now = DateTime::from_timestamp(fields.open_now, 0).ok_or(TimestampOutOfRange)?;
    match operation {
        SessionOperation::Bearer => {
            let result = session_credentials::resolve_session_bearer(&candidate, &open_keys, now);
            Ok(match result {
                SessionBearerResult::NotSessionBearer => json!({ "tag": "not_session_bearer" }),
                SessionBearerResult::SessionBearerAdmitted { principal, .. } => json!({
                    "tag": "admitted",
                    "principal": principal,
                }),
                SessionBearerResult::SessionBearerInvalid { expired } => json!({
                    "tag": "invalid",
                    "expired": expired,
                }),
            })
        }
        SessionOperation::Refresh => {
            let expected_client_id = fields
                .expected_client_id
                .clone()
                .ok_or(MissingExpectedClientId)?;
            let result = session_credentials::open_session_refresh_bearer(
                &candidate,
                &open_keys,
                now,
                &expected_client_id,
            );
            Ok(match result {
                SessionRefreshResult::SessionRefreshOpened { principal, jti } => json!({
                    "tag": "opened",
                    "principal": principal,
                    "jti": jti,
                }),
                SessionRefreshResult::SessionRefreshInvalid => json!({ "tag": "invalid" }),
            })
        }
    }
}
