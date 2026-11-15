use betterlite_mcp::credential_contracts::auth_config::id_jag::PrivateKeyJwtAuth;
use betterlite_mcp::credential_contracts::errors::{CredError, CredResult};
use jsonwebtoken::{Algorithm, AlgorithmFamily, EncodingKey, Header, encode};
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::ASSERTION_LIFETIME_SECONDS;

pub(super) const SIGNING_DETAIL: &str = "token exchange failed: could not sign the client assertion; check client_private_key and client_assertion_signing_alg";

#[derive(Serialize)]
struct AssertionClaims<'claim> {
    iss: &'claim str,
    sub: &'claim str,
    aud: &'claim str,
    jti: &'claim str,
    iat: i64,
    exp: i64,
}

pub(super) fn client_assertion(
    auth: &PrivateKeyJwtAuth,
    endpoint: &str,
    client_id: &str,
    now: i64,
    jti: &str,
) -> CredResult<String> {
    let algorithm = auth
        .signing_alg
        .parse::<Algorithm>()
        .map_err(|_error| CredError::Misconfigured(SIGNING_DETAIL.to_owned()))?;
    let key = encoding_key(auth, algorithm)
        .ok_or_else(|| CredError::Misconfigured(SIGNING_DETAIL.to_owned()))?;
    let mut header = Header::new(algorithm);
    header.kid.clone_from(&auth.key_id);
    let claims = AssertionClaims {
        iss: client_id,
        sub: client_id,
        aud: endpoint,
        jti,
        iat: now,
        exp: now.saturating_add(ASSERTION_LIFETIME_SECONDS),
    };
    encode(&header, &claims, &key)
        .map_err(|_error| CredError::Misconfigured(SIGNING_DETAIL.to_owned()))
}
fn encoding_key(auth: &PrivateKeyJwtAuth, algorithm: Algorithm) -> Option<EncodingKey> {
    let material = auth.private_key.expose_secret().as_bytes();
    let parsed = pem::parse(material).ok();
    match (parsed, algorithm.family()) {
        (_, AlgorithmFamily::Hmac) => Some(EncodingKey::from_secret(material)),
        (Some(block), AlgorithmFamily::Rsa) if block.tag() == "RSA PRIVATE KEY" => {
            Some(EncodingKey::from_rsa_der(block.contents()))
        }
        (Some(block), AlgorithmFamily::Rsa) => {
            let info = pkcs8::PrivateKeyInfoRef::try_from(block.contents()).ok()?;
            Some(EncodingKey::from_rsa_der(info.private_key.as_bytes()))
        }
        (Some(block), AlgorithmFamily::Ec) => Some(EncodingKey::from_ec_der(block.contents())),
        (Some(block), AlgorithmFamily::Ed) => Some(EncodingKey::from_ed_der(block.contents())),
        _ => None,
    }
}
