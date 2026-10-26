mod tests;
use crate::errors::SessionOpenError::{BadSignature, Expired, Malformed, NotASessionToken};
use crate::errors::{SessionMintError as MintError, SessionOpenError as OpenError};
use crate::types::{
    MintedSessionToken, OpenedSessionToken, SessionAudience, SessionPrincipal,
    SessionSigningKeys as SigningKeys, SessionTokenKind,
};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, UnparsedPublicKey};
use aws_lc_rs::{error::Unspecified, hmac};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::ser::{Formatter, Serializer};
use std::io::{self, Write};

pub const SESSION_TOKEN_PREFIX: &str = "llm_session_";
pub const SESSION_REFRESH_PREFIX: &str = "llm_srefresh_";
pub const SESSION_ISSUER: &str = "litellm-mcp-gateway";
pub const SESSION_TTL_SECONDS: i64 = 3_600;
pub const SESSION_REFRESH_TTL_SECONDS: i64 = 1_209_600;
pub const MAX_SESSION_TOKEN_BYTES: usize = 4_096;

pub trait SessionPrefix {
    fn is_session_token(&self) -> bool;
    fn is_session_refresh_token(&self) -> bool;
}

impl SessionPrefix for str {
    fn is_session_token(&self) -> bool {
        self.starts_with(SESSION_TOKEN_PREFIX)
    }

    fn is_session_refresh_token(&self) -> bool {
        self.starts_with(SESSION_REFRESH_PREFIX)
    }
}

impl SessionTokenKind {
    const fn wire_shape(self) -> (&'static str, i64) {
        match self {
            Self::Session => (SESSION_TOKEN_PREFIX, SESSION_TTL_SECONDS),
            Self::Refresh => (SESSION_REFRESH_PREFIX, SESSION_REFRESH_TTL_SECONDS),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    iat: i64,
    exp: i64,
    jti: String,
    kind: SessionTokenKind,
    user_id: String,
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_server_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    audience: Option<SessionAudience>,
    #[serde(skip_serializing_if = "Option::is_none")]
    team_id: Option<String>,
}

// PyJWT sorts header keys before signing, so LiteLLM's signatures cover them in this order
#[derive(Serialize, Deserialize)]
struct Header {
    alg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<String>,
    #[serde(skip_deserializing)]
    typ: &'static str,
}

// Python's json.dumps escapes everything outside printable ASCII, and the signature covers those bytes
struct AsciiFormatter;

impl Formatter for AsciiFormatter {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        for unit in fragment.encode_utf16() {
            match char::from_u32(u32::from(unit)) {
                Some(character @ ' '..='~') => write!(writer, "{character}")?,
                _ => write!(writer, "\\u{unit:04x}")?,
            }
        }
        Ok(())
    }
}

pub fn mint_session_token<R>(
    principal: &SessionPrincipal,
    kind: SessionTokenKind,
    keys: &SigningKeys,
    now: DateTime<Utc>,
    random: R,
) -> Result<MintedSessionToken, MintError>
where
    R: FnOnce() -> Result<[u8; 16], Unspecified>,
{
    if principal.user_id.is_empty() || principal.client_id.is_empty() {
        return Err(MintError::EmptyIdentity);
    }
    let (prefix, ttl_seconds) = kind.wire_shape();
    let expires_at = TimeDelta::new(ttl_seconds, 0)
        .and_then(|ttl| now.checked_add_signed(ttl))
        .ok_or(MintError::ExpiryOutOfRange)?;
    let entropy = random().map_err(MintError::Random)?;
    let claims = Claims {
        iss: SESSION_ISSUER.to_owned(),
        iat: now.timestamp(),
        exp: expires_at.timestamp(),
        jti: URL_SAFE_NO_PAD.encode(entropy),
        kind,
        user_id: principal.user_id.clone(),
        client_id: principal.client_id.clone(),
        resource_server_id: principal.resource_server_id.clone(),
        audience: principal.audience,
        team_id: principal.team_id.clone(),
    };
    let token = format!("{prefix}{}", sign(&claims, keys)?);
    if token.len() > MAX_SESSION_TOKEN_BYTES {
        return Err(MintError::TooLarge {
            size_bytes: token.len(),
            max_bytes: MAX_SESSION_TOKEN_BYTES,
        });
    }
    Ok(MintedSessionToken {
        token: SecretString::from(token),
        expires_at,
    })
}

fn python_json_segment<T>(value: &T) -> Result<String, MintError>
where
    T: Serialize,
{
    let mut serializer = Serializer::with_formatter(Vec::new(), AsciiFormatter);
    value.serialize(&mut serializer)?;
    Ok(URL_SAFE_NO_PAD.encode(serializer.into_inner()))
}

fn sign(claims: &Claims, keys: &SigningKeys) -> Result<String, MintError> {
    let (alg, kid) = match keys {
        SigningKeys::Symmetric(_) => ("HS256", None),
        SigningKeys::Asymmetric(asymmetric) => ("RS256", Some(asymmetric.kid.as_str())),
    };
    let header = python_json_segment(&Header {
        alg: Some(alg.to_owned()),
        kid: kid.map(str::to_owned),
        typ: "JWT",
    })?;
    let signing_input = format!("{header}.{}", python_json_segment(claims)?);
    let message = signing_input.as_bytes();
    let signature = match keys {
        SigningKeys::Symmetric(symmetric) => {
            let secret = symmetric.signing_key.expose_secret().as_bytes();
            let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret), message);
            URL_SAFE_NO_PAD.encode(tag)
        }
        SigningKeys::Asymmetric(asymmetric) => {
            let signer = &asymmetric.signer;
            let mut signature = vec![0; signer.public_modulus_len()];
            // aws-lc ignores the random source for PKCS#1 v1.5, which is deterministic
            let random = SystemRandom::new();
            signer
                .sign(&RSA_PKCS1_SHA256, &random, message, &mut signature)
                .map_err(MintError::Signature)?;
            URL_SAFE_NO_PAD.encode(signature)
        }
    };
    Ok(format!("{signing_input}.{signature}"))
}

pub fn open_session_token(
    candidate: &str,
    kind: SessionTokenKind,
    keys: &SigningKeys,
    now: DateTime<Utc>,
) -> Result<OpenedSessionToken, OpenError> {
    let (prefix, _) = kind.wire_shape();
    let compact = candidate.strip_prefix(prefix).ok_or(NotASessionToken)?;
    if candidate.len() > MAX_SESSION_TOKEN_BYTES {
        return Err(Malformed);
    }
    let claims = verified_claims(compact, keys)?;
    if claims.iss != SESSION_ISSUER
        || claims.jti.is_empty()
        || claims.user_id.is_empty()
        || claims.client_id.is_empty()
        || claims.kind != kind
    {
        return Err(Malformed);
    }
    if now.timestamp() >= claims.exp {
        return Err(Expired);
    }
    Ok(OpenedSessionToken {
        principal: SessionPrincipal {
            user_id: claims.user_id,
            client_id: claims.client_id,
            resource_server_id: claims.resource_server_id,
            audience: claims.audience,
            team_id: claims.team_id,
        },
        jti: claims.jti,
        kind,
        iat: claims.iat,
        exp: claims.exp,
    })
}

fn verified_claims(compact: &str, keys: &SigningKeys) -> Result<Claims, OpenError> {
    let decode = |segment: &str| URL_SAFE_NO_PAD.decode(segment).map_err(|_source| Malformed);
    let (signing_input, signature_segment) = compact.rsplit_once('.').ok_or(Malformed)?;
    let (header_segment, payload_segment) = signing_input.split_once('.').ok_or(Malformed)?;
    let header: Header =
        serde_json::from_slice(&decode(header_segment)?).map_err(|_source| Malformed)?;
    let payload = decode(payload_segment)?;
    let signature = decode(signature_segment)?;
    let message = signing_input.as_bytes();
    let (algorithm, verified) = match keys {
        SigningKeys::Symmetric(symmetric) => {
            let secret = symmetric.signing_key.expose_secret().as_bytes();
            let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
            ("HS256", hmac::verify(&key, message, &signature).is_ok())
        }
        SigningKeys::Asymmetric(asymmetric) => {
            let public = asymmetric.verification_key(header.kid.as_deref())?;
            let key = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public);
            ("RS256", key.verify(message, &signature).is_ok())
        }
    };
    if header.alg.as_deref() != Some(algorithm) {
        return Err(Malformed);
    }
    if !verified {
        return Err(BadSignature);
    }
    serde_json::from_slice(&payload).map_err(|_source| Malformed)
}
