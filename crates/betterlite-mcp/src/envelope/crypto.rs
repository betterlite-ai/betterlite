use aws_lc_rs::hmac;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue,
};
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    ENVELOPE_ISSUER, MAX_ENVELOPE_BYTES,
    models::{Claims, EnvelopeKeys, EnvelopeMintError, EnvelopeOpenError, Header, HeaderOwned},
};

pub(crate) fn encode_jwt(claims: &Claims, key: &SecretString) -> Result<String, serde_json::Error> {
    let header = serde_json::to_vec(&Header {
        alg: "HS256",
        typ: "JWT",
    })?;
    let payload = serde_json::to_vec(claims)?;
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let tag = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, key.expose_secret().as_bytes()),
        signing_input.as_bytes(),
    );
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    ))
}

pub(crate) fn open_claims(
    candidate: &str,
    prefix: &str,
    expected_kind: &str,
    keys: &EnvelopeKeys,
    now: DateTime<Utc>,
) -> Result<Claims, EnvelopeOpenError> {
    let compact = candidate
        .strip_prefix(prefix)
        .ok_or(EnvelopeOpenError::NotAnEnvelope)?;
    if candidate.len() > MAX_ENVELOPE_BYTES {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    let claims = verify_jwt(compact, &keys.signing_key)?;
    let valid = claims.kind == expected_kind
        && claims.iss == ENVELOPE_ISSUER
        && !claims.server_id.is_empty()
        && !claims.subject_type.is_empty()
        && !claims.subject.is_empty()
        && !claims.grant.is_empty();
    if !valid {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    if now.timestamp() >= claims.exp {
        return Err(EnvelopeOpenError::Expired);
    }
    Ok(claims)
}

fn verify_jwt(compact: &str, key: &SecretString) -> Result<Claims, EnvelopeOpenError> {
    let (signing_input, signature_segment) = compact
        .rsplit_once('.')
        .ok_or(EnvelopeOpenError::MalformedPayload)?;
    let (header_segment, payload_segment) = signing_input
        .split_once('.')
        .ok_or(EnvelopeOpenError::MalformedPayload)?;
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    let header: HeaderOwned = serde_json::from_slice(&header_bytes)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    if header.alg != "HS256" || header.typ != "JWT" {
        return Err(EnvelopeOpenError::MalformedPayload);
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, key.expose_secret().as_bytes());
    if hmac::verify(&hmac_key, signing_input.as_bytes(), &signature).is_err() {
        return Err(EnvelopeOpenError::BadSignature);
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload_segment)
        .map_err(|_error| EnvelopeOpenError::MalformedPayload)?;
    serde_json::from_slice(&payload).map_err(|_error| EnvelopeOpenError::MalformedPayload)
}

pub(crate) fn encrypt_grant(
    plaintext: &str,
    key: &SecretString,
) -> Result<String, EnvelopeMintError> {
    let encrypted = cipher(key)
        .encrypt_value(StoredValue::<()>::Text(SecretString::from(plaintext)), None)
        .map_err(|_error| EnvelopeMintError::EncryptionFailed)?;
    let StoredValue::Text(sealed) = encrypted else {
        return Err(EnvelopeMintError::EncryptionFailed);
    };
    Ok(URL_SAFE_NO_PAD.encode(sealed.expose_secret()))
}

pub(crate) fn decrypt_text(
    encoded: &str,
    key: &SecretString,
    field: &str,
) -> Result<String, EnvelopeOpenError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let text = String::from_utf8(decoded).map_err(|_error| EnvelopeOpenError::DecryptFailed)?;
    let opened = cipher(key).decrypt_value(
        StoredValue::<()>::Text(SecretString::from(text)),
        field,
        DecryptPolicy {
            log: FailureLog::Debug,
            value: FailureValue::Nothing,
        },
    );
    let Some(StoredValue::Text(plaintext)) = opened else {
        return Err(EnvelopeOpenError::DecryptFailed);
    };
    Ok(plaintext.expose_secret().to_owned())
}

struct NoopDiagnostics;

impl SecretDiagnostics for NoopDiagnostics {
    fn record(&self, _level: Level, _event: &SecretEvent<'_>) {}
}

fn cipher(key: &SecretString) -> StoredCipher<OsNonces, NoopDiagnostics> {
    let settings = CipherSettings {
        signing_key: Some(key.clone()),
        algorithm: Algorithm::XSalsa20Poly1305,
    };
    StoredCipher {
        settings,
        nonces: OsNonces,
        diagnostics: NoopDiagnostics,
    }
}
