use serde::Deserialize;
use serde_json::json;

use betterlite_mcp::bridge_credentials::{self, BridgeEnvelopeResult, BridgeRefreshResult};
use betterlite_mcp::envelope::{
    EnvelopeIdentity, EnvelopeKeys, RefreshCredential, UpstreamTokenGrant, mint_envelope,
    mint_refresh_envelope,
};
use betterlite_secrets::secret_codec::OsNonces;
use secrecy::{ExposeSecret as _, SecretString};

use crate::helpers::{HelperError, HelperResult};

use super::{at, helper_error};

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
    access_token: Option<String>,
    token_type: Option<String>,
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

fn envelope_keys(signing: &str, encryption: &str) -> Result<EnvelopeKeys, Box<HelperError>> {
    let keys = EnvelopeKeys::new(SecretString::from(signing), SecretString::from(encryption));
    keys.map_err(|error| helper_error("EnvelopeMintError", &error))
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

fn mint_access_candidate(
    mint: &EnvelopeMint,
    keys: &EnvelopeKeys,
) -> Result<String, Box<HelperError>> {
    let grant = UpstreamTokenGrant {
        access_token: SecretString::from(mint.access_token.clone().unwrap_or_default()),
        token_type: mint.token_type.clone().unwrap_or_default(),
        refresh_token: mint.refresh_token.clone().map(SecretString::from),
        scope: mint.scope.clone(),
        expires_in: mint.expires_in,
    };
    let now = at(mint.mint_now)?;
    let sealed = mint_envelope(identity(mint), grant, keys, now, &OsNonces)
        .map_err(|error| helper_error("EnvelopeMintError", &error))?;
    Ok(sealed.token.expose_secret().to_owned())
}

fn mint_refresh_candidate(
    mint: &EnvelopeMint,
    keys: &EnvelopeKeys,
) -> Result<String, Box<HelperError>> {
    let refresh = RefreshCredential {
        refresh_token: SecretString::from(mint.refresh_token.clone().unwrap_or_default()),
        scope: mint.scope.clone(),
        expires_in: mint.expires_in,
    };
    let now = at(mint.mint_now)?;
    let sealed = mint_refresh_envelope(identity(mint), refresh, keys, now, &OsNonces)
        .map_err(|error| helper_error("EnvelopeMintError", &error))?;
    Ok(sealed.token.expose_secret().to_owned())
}

pub(crate) fn run_bridge_envelope(fields: &EnvelopeFields) -> HelperResult {
    let mint_keys = envelope_keys(&fields.signing_key, &fields.encryption_key)?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => mint_access_candidate(mint, &mint_keys)?,
        (None, Some(literal)) => literal.clone(),
        (None, None) => {
            return Err(helper_error(
                "ValueError",
                &"either mint or candidate is required",
            ));
        }
    };
    let open_keys = match (&fields.open_signing_key, &fields.open_encryption_key) {
        (Some(signing), Some(encryption)) => envelope_keys(signing, encryption)?,
        _ => mint_keys,
    };
    let now = at(fields.open_now)?;
    let result = bridge_credentials::resolve_bridge_envelope(
        &candidate,
        &open_keys,
        now,
        &fields.expected_server_id,
    );
    Ok(match result {
        BridgeEnvelopeResult::NotBridgeEnvelope => json!({ "tag": "not_bridge_envelope" }),
        BridgeEnvelopeResult::BridgeEnvelopeAdmitted {
            identity,
            upstream_authorization,
        } => json!({
            "tag": "admitted",
            "identity": identity,
            "upstream_authorization": upstream_authorization.expose_secret(),
        }),
        BridgeEnvelopeResult::BridgeEnvelopeInvalid => json!({ "tag": "invalid" }),
    })
}

pub(crate) fn run_bridge_refresh(fields: &EnvelopeFields) -> HelperResult {
    let mint_keys = envelope_keys(&fields.signing_key, &fields.encryption_key)?;
    let candidate = match (&fields.mint, &fields.candidate) {
        (Some(mint), _) => mint_refresh_candidate(mint, &mint_keys)?,
        (None, Some(literal)) => literal.clone(),
        (None, None) => {
            return Err(helper_error(
                "ValueError",
                &"either mint or candidate is required",
            ));
        }
    };
    let open_keys = match (&fields.open_signing_key, &fields.open_encryption_key) {
        (Some(signing), Some(encryption)) => envelope_keys(signing, encryption)?,
        _ => mint_keys,
    };
    let now = at(fields.open_now)?;
    let result = bridge_credentials::open_bridge_refresh_envelope(
        &candidate,
        &open_keys,
        now,
        &fields.expected_server_id,
    );
    Ok(match result {
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
    })
}
