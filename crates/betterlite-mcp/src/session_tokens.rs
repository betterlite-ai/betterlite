mod tests;

use std::io::{self, Write};

use aws_lc_rs::hmac;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::ser::{Formatter, Serializer};

use crate::errors::{SessionMintError, SessionOpenError};
use crate::types::{
    MintedSessionToken, OpenedSessionToken, SessionAudience, SessionPrincipal, SessionSigningKeys,
    SessionTokenKind,
};

pub const SESSION_TOKEN_PREFIX: &str = "llm_session_";
pub const SESSION_REFRESH_PREFIX: &str = "llm_srefresh_";
pub const SESSION_ISSUER: &str = "litellm-mcp-gateway";
pub const SESSION_TTL_SECONDS: i64 = 3_600;
pub const SESSION_REFRESH_TTL_SECONDS: i64 = 1_209_600;
pub const MAX_SESSION_TOKEN_BYTES: usize = 4_096;

pub trait JtiSource {
    fn fill(&self, jti: &mut [u8; 16]) -> Result<(), getrandom::Error>;
}

pub struct OsJti;

impl JtiSource for OsJti {
    fn fill(&self, jti: &mut [u8; 16]) -> Result<(), getrandom::Error> {
        getrandom::fill(jti)
    }
}

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
#[derive(Serialize)]
struct SignedHeader<'kid> {
    alg: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kid: Option<&'kid str>,
    typ: &'static str,
}

#[derive(Deserialize)]
struct PresentedHeader {
    alg: Option<String>,
    kid: Option<String>,
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
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
    random: &R,
) -> Result<MintedSessionToken, SessionMintError>
where
    R: JtiSource,
{
    if principal.user_id.is_empty() || principal.client_id.is_empty() {
        return Err(SessionMintError::EmptyIdentity);
    }
    let (prefix, ttl_seconds) = kind.wire_shape();
    let expires_at = TimeDelta::new(ttl_seconds, 0)
        .and_then(|ttl| now.checked_add_signed(ttl))
        .ok_or(SessionMintError::ExpiryOutOfRange)?;
    let mut entropy = [0_u8; 16];
    random
        .fill(&mut entropy)
        .map_err(|source| SessionMintError::Random { source })?;
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
        return Err(SessionMintError::TooLarge {
            size_bytes: token.len(),
            max_bytes: MAX_SESSION_TOKEN_BYTES,
        });
    }
    Ok(MintedSessionToken {
        token: SecretString::from(token),
        expires_at,
    })
}

fn python_json_segment<T>(value: &T) -> Result<String, SessionMintError>
where
    T: Serialize,
{
    let mut serializer = Serializer::with_formatter(Vec::new(), AsciiFormatter);
    value
        .serialize(&mut serializer)
        .map_err(|source| SessionMintError::Claims { source })?;
    Ok(URL_SAFE_NO_PAD.encode(serializer.into_inner()))
}

fn sign(claims: &Claims, keys: &SessionSigningKeys) -> Result<String, SessionMintError> {
    let (alg, kid) = match keys {
        SessionSigningKeys::Symmetric(_) => ("HS256", None),
        SessionSigningKeys::Asymmetric(asymmetric) => ("RS256", Some(asymmetric.kid.as_str())),
    };
    let header = python_json_segment(&SignedHeader {
        alg,
        kid,
        typ: "JWT",
    })?;
    let signing_input = format!("{header}.{}", python_json_segment(claims)?);
    let message = signing_input.as_bytes();
    let signature = match keys {
        SessionSigningKeys::Symmetric(symmetric) => {
            let secret = symmetric.signing_key.expose_secret().as_bytes();
            let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, secret), message);
            tag.as_ref().to_vec()
        }
        SessionSigningKeys::Asymmetric(asymmetric) => {
            let signer = &asymmetric.signer;
            let mut signature = vec![0; signer.public_modulus_len()];
            // aws-lc ignores the random source for PKCS#1 v1.5, which is deterministic
            let random = SystemRandom::new();
            signer
                .sign(&RSA_PKCS1_SHA256, &random, message, &mut signature)
                .map_err(|source| SessionMintError::Signature { source })?;
            signature
        }
    };
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

pub fn open_session_token(
    candidate: &str,
    kind: SessionTokenKind,
    keys: &SessionSigningKeys,
    now: DateTime<Utc>,
) -> Result<OpenedSessionToken, SessionOpenError> {
    let (prefix, _) = kind.wire_shape();
    let compact = candidate
        .strip_prefix(prefix)
        .ok_or(SessionOpenError::NotASessionToken)?;
    if candidate.len() > MAX_SESSION_TOKEN_BYTES {
        return Err(SessionOpenError::Malformed);
    }
    let claims = verified_claims(compact, keys)?;
    if claims.iss != SESSION_ISSUER
        || claims.jti.is_empty()
        || claims.user_id.is_empty()
        || claims.client_id.is_empty()
        || claims.kind != kind
    {
        return Err(SessionOpenError::Malformed);
    }
    if now.timestamp() >= claims.exp {
        return Err(SessionOpenError::Expired);
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

fn verified_claims(compact: &str, keys: &SessionSigningKeys) -> Result<Claims, SessionOpenError> {
    let decode = |segment: &str| {
        URL_SAFE_NO_PAD
            .decode(segment)
            .map_err(|_source| SessionOpenError::Malformed)
    };
    let (signing_input, signature_segment) = compact
        .rsplit_once('.')
        .ok_or(SessionOpenError::Malformed)?;
    let (header_segment, payload_segment) = signing_input
        .split_once('.')
        .ok_or(SessionOpenError::Malformed)?;
    let header: PresentedHeader = serde_json::from_slice(&decode(header_segment)?)
        .map_err(|_source| SessionOpenError::Malformed)?;
    let payload = decode(payload_segment)?;
    let signature = decode(signature_segment)?;
    let message = signing_input.as_bytes();
    let (algorithm, verified) = match keys {
        SessionSigningKeys::Symmetric(symmetric) => {
            let secret = symmetric.signing_key.expose_secret().as_bytes();
            let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
            ("HS256", hmac::verify(&key, message, &signature).is_ok())
        }
        SessionSigningKeys::Asymmetric(asymmetric) => {
            let public = asymmetric.verification_key(header.kid.as_deref())?;
            let key = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public);
            ("RS256", key.verify(message, &signature).is_ok())
        }
    };
    if header.alg.as_deref() != Some(algorithm) {
        return Err(SessionOpenError::Malformed);
    }
    if !verified {
        return Err(SessionOpenError::BadSignature);
    }
    serde_json::from_slice(&payload).map_err(|_source| SessionOpenError::Malformed)
}
