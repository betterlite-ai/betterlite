use crate::envelope::{
    EnvelopeIdentity, EnvelopeKeys, EnvelopeMintError, EnvelopePrefix as _, RefreshCredential,
    open_envelope, open_refresh_envelope,
};
use chrono::{DateTime, Utc};
use scrypt::{Params, scrypt};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

pub const SIGNING_KEY_DOMAIN: &[u8] = b"litellm-mcp-bridge:envelope-signing:";
pub const ENCRYPTION_KEY_DOMAIN: &[u8] = b"litellm-mcp-bridge:envelope-encryption:";
const SCRYPT_LOG_N: u8 = 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const DERIVED_KEY_BYTES: usize = 32;

#[derive(Debug, Error)]
pub enum BridgeKeyError {
    #[error("scrypt parameters are invalid")]
    ScryptParams,
    #[error("scrypt output length is invalid")]
    ScryptOutput,
    #[error("derived envelope keys are invalid")]
    Keys(#[from] EnvelopeMintError),
}

// rotating master_key invalidates every outstanding envelope, since both subkeys derive from it.
pub fn envelope_keys_from_master_key(
    master_key: &SecretString,
) -> Result<EnvelopeKeys, BridgeKeyError> {
    let signing = scrypt_hex(master_key.expose_secret(), SIGNING_KEY_DOMAIN)?;
    let encryption = scrypt_hex(master_key.expose_secret(), ENCRYPTION_KEY_DOMAIN)?;
    Ok(EnvelopeKeys::new(
        SecretString::from(signing),
        SecretString::from(encryption),
    )?)
}

// exposed so the oracle can compare it against LiteLLM's hashlib.scrypt(...).hex().
pub fn scrypt_hex(master_key: &str, domain: &[u8]) -> Result<String, BridgeKeyError> {
    let params = Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, DERIVED_KEY_BYTES)
        .map_err(|_scrypt_error| BridgeKeyError::ScryptParams)?;
    let mut output = [0_u8; DERIVED_KEY_BYTES];
    scrypt(master_key.as_bytes(), domain, &params, &mut output)
        .map_err(|_scrypt_error| BridgeKeyError::ScryptOutput)?;
    Ok(hex::encode(output))
}

#[derive(Debug, Clone)]
pub enum BridgeRefreshResult {
    BridgeRefreshOpened {
        identity: EnvelopeIdentity,
        refresh: RefreshCredential,
    },
    BridgeRefreshInvalid,
}

// a raw upstream refresh token is BridgeRefreshInvalid too: this mode never hands the client a bare one.
#[must_use]
pub fn open_bridge_refresh_envelope(
    refresh_value: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
    expected_server_id: &str,
) -> BridgeRefreshResult {
    let candidate = strip_bearer(refresh_value);
    if !candidate.is_refresh_envelope() {
        return BridgeRefreshResult::BridgeRefreshInvalid;
    }
    let Ok(opened) = open_refresh_envelope(candidate, keys, now) else {
        return BridgeRefreshResult::BridgeRefreshInvalid;
    };
    if opened.identity.server_id != expected_server_id {
        return BridgeRefreshResult::BridgeRefreshInvalid;
    }
    BridgeRefreshResult::BridgeRefreshOpened {
        identity: opened.identity,
        refresh: opened.refresh,
    }
}

#[derive(Debug, Clone)]
pub enum BridgeEnvelopeResult {
    NotBridgeEnvelope,
    BridgeEnvelopeAdmitted {
        identity: EnvelopeIdentity,
        upstream_authorization: SecretString,
    },
    BridgeEnvelopeInvalid,
}

fn strip_bearer(value: &str) -> &str {
    let Some((scheme, rest)) = value.split_once(char::is_whitespace) else {
        return value;
    };
    if scheme.eq_ignore_ascii_case("bearer") {
        rest.trim_start()
    } else {
        value
    }
}

// a plain upstream bearer falls through to normal oauth2 admission instead of matching here.
#[must_use]
pub fn is_bridge_envelope_shaped(authorization_value: &str) -> bool {
    let candidate = strip_bearer(authorization_value);
    candidate.is_envelope() || candidate.is_refresh_envelope()
}

// a refresh envelope is BridgeEnvelopeInvalid here too: it authenticates nothing but a token-endpoint call.
#[must_use]
pub fn resolve_bridge_envelope(
    authorization_value: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
    expected_server_id: &str,
) -> BridgeEnvelopeResult {
    let candidate = strip_bearer(authorization_value);
    if candidate.is_refresh_envelope() {
        return BridgeEnvelopeResult::BridgeEnvelopeInvalid;
    }
    if !candidate.is_envelope() {
        return BridgeEnvelopeResult::NotBridgeEnvelope;
    }
    let Ok(opened) = open_envelope(candidate, keys, now) else {
        return BridgeEnvelopeResult::BridgeEnvelopeInvalid;
    };
    if opened.identity.server_id != expected_server_id {
        return BridgeEnvelopeResult::BridgeEnvelopeInvalid;
    }
    let grant = opened.grant;
    let authorization_scheme = if grant.token_type.eq_ignore_ascii_case("bearer") {
        "Bearer"
    } else {
        grant.token_type.as_str()
    };
    let upstream_authorization = SecretString::from(format!(
        "{authorization_scheme} {}",
        grant.access_token.expose_secret()
    ));
    BridgeEnvelopeResult::BridgeEnvelopeAdmitted {
        identity: opened.identity,
        upstream_authorization,
    }
}

mod tests;
