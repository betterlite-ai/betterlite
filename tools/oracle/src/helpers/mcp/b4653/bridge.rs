use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use betterlite_mcp::bridge_credentials::{
    BridgeEnvelopeResult, BridgeRefreshResult, open_bridge_refresh_envelope,
    resolve_bridge_envelope,
};
use betterlite_mcp::envelope::{
    EnvelopeIdentity, EnvelopeKeys, EnvelopeMintError, RefreshCredential, SealedEnvelope,
    UpstreamTokenGrant, mint_envelope, mint_refresh_envelope,
};
use betterlite_mcp::errors::{SessionKeyError, SessionMintError};
use betterlite_secrets::secret_codec::OsNonces;
use secrecy::{ExposeSecret as _, SecretString};

use crate::helpers::{HelperError, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvelopeFields {
    signing_key: String,
    encryption_key: String,
    open_signing_key: Option<String>,
    open_encryption_key: Option<String>,
    candidate: Option<String>,
    mint: Option<EnvelopeMint>,
    open_now: i64,
    expected_server_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvelopeMint {
    server_id: String,
    subject_type: SubjectTypeInput,
    subject: String,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    token_type: String,
    refresh_token: Option<String>,
    scope: Option<String>,
    expires_in: Option<i64>,
    mint_now: i64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubjectTypeInput {
    KeyHash,
    UserId,
}

#[derive(Debug, Error)]
pub(crate) enum Failure {
    #[error("timestamp out of range")]
    TimestampOutOfRange,
    #[error("either mint or candidate is required")]
    MissingMintOrCandidate,
    #[error("expected_client_id is required")]
    MissingExpectedClientId,
    #[error(transparent)]
    EnvelopeMint(#[from] EnvelopeMintError),
    #[error(transparent)]
    SessionKey(#[from] SessionKeyError),
    #[error(transparent)]
    SessionMint(#[from] SessionMintError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::TimestampOutOfRange
            | Failure::MissingMintOrCandidate
            | Failure::MissingExpectedClientId => "ValueError",
            Failure::EnvelopeMint(_) => "EnvelopeMintError",
            Failure::SessionKey(_) => "SessionKeyError",
            Failure::SessionMint(_) => "SessionMintError",
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

fn identity(mint: &EnvelopeMint) -> EnvelopeIdentity {
    let subject_type = match mint.subject_type {
        SubjectTypeInput::KeyHash => betterlite_mcp::envelope::models::SubjectType::KeyHash,
        SubjectTypeInput::UserId => betterlite_mcp::envelope::models::SubjectType::UserId,
    };
    EnvelopeIdentity {
        server_id: mint.server_id.clone(),
        subject_type,
        subject: mint.subject.clone(),
    }
}

fn seal_candidate<P, S>(
    mint: &EnvelopeMint,
    keys: &EnvelopeKeys,
    payload: P,
    seal: S,
) -> Result<String, Failure>
where
    S: FnOnce(
        EnvelopeIdentity,
        P,
        &EnvelopeKeys,
        DateTime<Utc>,
        &OsNonces,
    ) -> Result<SealedEnvelope, EnvelopeMintError>,
{
    let now = DateTime::from_timestamp(mint.mint_now, 0).ok_or(Failure::TimestampOutOfRange)?;
    let sealed = seal(identity(mint), payload, keys, now, &OsNonces)?;
    Ok(sealed.token.expose_secret().to_owned())
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BridgeOperation {
    Envelope,
    Refresh,
}

fn candidate_and_open_keys<P, F, S>(
    fields: &EnvelopeFields,
    build_payload: F,
    seal: S,
) -> Result<(String, EnvelopeKeys), Failure>
where
    F: FnOnce(&EnvelopeMint) -> P,
    S: FnOnce(
        EnvelopeIdentity,
        P,
        &EnvelopeKeys,
        DateTime<Utc>,
        &OsNonces,
    ) -> Result<SealedEnvelope, EnvelopeMintError>,
{
    let mint_keys = EnvelopeKeys::new(
        SecretString::from(fields.signing_key.as_str()),
        SecretString::from(fields.encryption_key.as_str()),
    )?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => seal_candidate(mint, &mint_keys, build_payload(mint), seal)?,
        (None, Some(literal)) => literal.clone(),
        (None, None) => return Err(Failure::MissingMintOrCandidate),
    };
    let open_keys = match (&fields.open_signing_key, &fields.open_encryption_key) {
        (Some(signing), Some(encryption)) => EnvelopeKeys::new(
            SecretString::from(signing.as_str()),
            SecretString::from(encryption.as_str()),
        )?,
        _ => mint_keys,
    };
    Ok((candidate, open_keys))
}

fn open_credential<P, S, R>(
    fields: &EnvelopeFields,
    now: DateTime<Utc>,
    build_payload: fn(&EnvelopeMint) -> P,
    seal: S,
    resolve: fn(&str, &EnvelopeKeys, DateTime<Utc>, &str) -> R,
) -> Result<R, Failure>
where
    S: FnOnce(
        EnvelopeIdentity,
        P,
        &EnvelopeKeys,
        DateTime<Utc>,
        &OsNonces,
    ) -> Result<SealedEnvelope, EnvelopeMintError>,
{
    let (candidate, open_keys) = candidate_and_open_keys(fields, build_payload, seal)?;
    Ok(resolve(
        &candidate,
        &open_keys,
        now,
        &fields.expected_server_id,
    ))
}

fn resolve_outcome(
    operation: BridgeOperation,
    candidate: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
    expected_server_id: &str,
) -> serde_json::Value {
    match operation {
        BridgeOperation::Envelope => {
            match resolve_bridge_envelope(candidate, keys, now, expected_server_id) {
                BridgeEnvelopeResult::NotBridgeEnvelope => {
                    json!({ "tag": "not_bridge_envelope" })
                }
                BridgeEnvelopeResult::BridgeEnvelopeAdmitted {
                    identity,
                    upstream_authorization,
                } => json!({
                    "tag": "admitted",
                    "identity": identity,
                    "upstream_authorization": upstream_authorization.expose_secret(),
                }),
                BridgeEnvelopeResult::BridgeEnvelopeInvalid => json!({ "tag": "invalid" }),
            }
        }
        BridgeOperation::Refresh => {
            match open_bridge_refresh_envelope(candidate, keys, now, expected_server_id) {
                BridgeRefreshResult::BridgeRefreshOpened { identity, refresh } => json!({
                    "tag": "opened",
                    "identity": identity,
                    "refresh": {
                        "refresh_token": refresh.refresh_token.expose_secret(),
                        "scope": refresh.scope,
                        "expires_in": refresh.expires_in,
                    },
                }),
                BridgeRefreshResult::BridgeRefreshInvalid => json!({ "tag": "invalid" }),
            }
        }
    }
}

pub(crate) fn run_bridge(fields: &EnvelopeFields, operation: BridgeOperation) -> HelperResult {
    let now = DateTime::from_timestamp(fields.open_now, 0).ok_or(Failure::TimestampOutOfRange)?;
    let result = match operation {
        BridgeOperation::Envelope => open_credential(
            fields,
            now,
            |mint: &EnvelopeMint| UpstreamTokenGrant {
                access_token: SecretString::from(mint.access_token.clone()),
                token_type: mint.token_type.clone(),
                refresh_token: mint.refresh_token.clone().map(SecretString::from),
                scope: mint.scope.clone(),
                expires_in: mint.expires_in,
            },
            mint_envelope,
            |cand, keys, at, srv| resolve_outcome(BridgeOperation::Envelope, cand, keys, at, srv),
        ),
        BridgeOperation::Refresh => open_credential(
            fields,
            now,
            |mint: &EnvelopeMint| RefreshCredential {
                refresh_token: SecretString::from(mint.refresh_token.clone().unwrap_or_default()),
                scope: mint.scope.clone(),
                expires_in: mint.expires_in,
            },
            mint_refresh_envelope,
            |cand, keys, at, srv| resolve_outcome(BridgeOperation::Refresh, cand, keys, at, srv),
        ),
    };
    Ok(result?)
}
