use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use betterlite_prompts::arguments::json::Json;
use jsonwebtoken::errors::Error as JsonWebTokenError;
use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use thiserror::Error;

use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer::idp_documents::IdpError;
use crate::mcp_jwt_signer::signer_settings::SignerSettings;
use crate::mcp_token_claims::Claims;

mod tests;

const EMPTY_KID_TEXT: &str = "none";

#[derive(Debug, Error)]
pub enum IncomingTokenError {
    #[error("discovery document at {uri} has no 'jwks_uri'")]
    MissingJwksUri { uri: String },
    #[error("the incoming token header is not base64url: {source}")]
    HeaderBase64 {
        #[source]
        source: base64::DecodeError,
    },
    #[error("the incoming token header is not a JSON object: {source}")]
    HeaderJson {
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "Failed to parse JWKS from {uri}: key {index} uses algorithm {algorithm}, which is not supported"
    )]
    UnsupportedAlgorithm {
        uri: String,
        index: usize,
        algorithm: String,
    },
    #[error("Failed to parse JWKS from {uri}: key {index} has an unsupported key type")]
    UnsupportedKeyType { uri: String, index: usize },
    #[error("Failed to parse JWKS from {uri}: key {index} cannot be decoded: {source}")]
    UndecodableKey {
        uri: String,
        index: usize,
        #[source]
        source: JsonWebTokenError,
    },
    #[error("No JWKS key matching kid={kid} at {uri}")]
    MissingKey { kid: String, uri: String },
    #[error("the incoming token failed verification: {source}")]
    Verification {
        #[source]
        source: JsonWebTokenError,
    },
    #[error("the incoming token's {claim} claim must be {expected}")]
    ClaimType {
        claim: &'static str,
        expected: &'static str,
    },
    #[error("the incoming token is not yet valid")]
    NotYetValid,
    #[error(transparent)]
    Idp(#[from] IdpError),
}

struct SigningJwk {
    kid: Option<String>,
    algorithm: Algorithm,
    decoding: DecodingKey,
}

#[derive(Debug, Deserialize)]
struct UnverifiedHeader {
    #[serde(default)]
    kid: Option<String>,
}

pub async fn verify_incoming_jwt(
    signer: &JwtSigner,
    raw_token: &SecretString,
    now: i64,
) -> Result<Claims, IncomingTokenError> {
    let discovery_uri = signer.settings.access_token_discovery_uri.as_deref();
    let discovery = signer.idp.get_oidc_discovery(discovery_uri, now).await?;
    let Some(jwks_uri) = discovery.jwks_uri.filter(|uri| !uri.is_empty()) else {
        return Err(IncomingTokenError::MissingJwksUri {
            uri: discovery_uri.unwrap_or_default().to_owned(),
        });
    };
    let keys = signer.idp.fetch_jwks(&jwks_uri, now).await?;
    let signing_keys = parse_keys(&keys, &jwks_uri)?;
    let kid = read_kid(raw_token.expose_secret())?;
    let selected = select_key(&signing_keys, kid.as_deref()).ok_or_else(|| {
        IncomingTokenError::MissingKey {
            kid: kid.clone().unwrap_or_else(|| EMPTY_KID_TEXT.to_owned()),
            uri: jwks_uri.clone(),
        }
    })?;
    decode(raw_token.expose_secret(), selected, &signer.settings, now)
}

fn decode(
    token: &str,
    key: &SigningJwk,
    settings: &SignerSettings,
    now: i64,
) -> Result<Claims, IncomingTokenError> {
    let mut validation = Validation::new(key.algorithm);
    validation.required_spec_claims.clear();
    validation.leeway = 0;
    validation.validate_nbf = true;
    match settings.verify_audience.as_deref() {
        Some(audience) => {
            validation.set_audience(&[audience]);
            validation.required_spec_claims.insert("aud".to_owned());
        }
        None => validation.validate_aud = false,
    }
    if let Some(issuer) = settings.verify_issuer.as_deref() {
        validation.set_issuer(&[issuer]);
        validation.required_spec_claims.insert("iss".to_owned());
    }
    let claims = jsonwebtoken::decode::<Claims>(token, &key.decoding, &validation)
        .map_err(|source| IncomingTokenError::Verification { source })?
        .claims;
    validate_claim_types(&claims, now)?;
    Ok(claims)
}

fn validate_claim_types(claims: &Claims, now: i64) -> Result<(), IncomingTokenError> {
    if claims
        .get("sub")
        .is_some_and(|value| !matches!(value, Json::Text(_)))
    {
        return Err(IncomingTokenError::ClaimType {
            claim: "sub",
            expected: "a string",
        });
    }
    if claims
        .get("jti")
        .is_some_and(|value| !matches!(value, Json::Text(_)))
    {
        return Err(IncomingTokenError::ClaimType {
            claim: "jti",
            expected: "a string",
        });
    }
    let Some(value) = claims.get("iat") else {
        return Ok(());
    };
    let Json::Number(number) = value else {
        return Err(IncomingTokenError::ClaimType {
            claim: "iat",
            expected: "an integer",
        });
    };
    let Some(issued_at) = number.as_i64() else {
        return Err(IncomingTokenError::ClaimType {
            claim: "iat",
            expected: "an integer",
        });
    };
    if issued_at > now {
        return Err(IncomingTokenError::NotYetValid);
    }
    Ok(())
}

fn parse_keys(keys: &[Jwk], uri: &str) -> Result<Vec<SigningJwk>, IncomingTokenError> {
    let mut parsed = Vec::with_capacity(keys.len());
    for (index, jwk) in keys.iter().enumerate() {
        let algorithm = key_algorithm(jwk, uri, index)?;
        let decoding =
            DecodingKey::from_jwk(jwk).map_err(|source| IncomingTokenError::UndecodableKey {
                uri: uri.to_owned(),
                index,
                source,
            })?;
        parsed.push(SigningJwk {
            kid: jwk.common.key_id.clone(),
            algorithm,
            decoding,
        });
    }
    Ok(parsed)
}

fn key_algorithm(jwk: &Jwk, uri: &str, index: usize) -> Result<Algorithm, IncomingTokenError> {
    if let Some(algorithm) = jwk.common.key_algorithm {
        return Algorithm::try_from(algorithm).map_err(|_source| {
            IncomingTokenError::UnsupportedAlgorithm {
                uri: uri.to_owned(),
                index,
                algorithm: algorithm.to_string(),
            }
        });
    }
    match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => Ok(Algorithm::RS256),
        AlgorithmParameters::EllipticCurve(parameters) => match parameters.curve {
            EllipticCurve::P256 => Ok(Algorithm::ES256),
            EllipticCurve::P384 => Ok(Algorithm::ES384),
            EllipticCurve::P521 | EllipticCurve::Ed25519 | _ => {
                Err(IncomingTokenError::UnsupportedKeyType {
                    uri: uri.to_owned(),
                    index,
                })
            }
        },
        AlgorithmParameters::OctetKey(_) => Ok(Algorithm::HS256),
        AlgorithmParameters::OctetKeyPair(parameters) => match parameters.curve {
            EllipticCurve::Ed25519 => Ok(Algorithm::EdDSA),
            EllipticCurve::P256 | EllipticCurve::P384 | EllipticCurve::P521 | _ => {
                Err(IncomingTokenError::UnsupportedKeyType {
                    uri: uri.to_owned(),
                    index,
                })
            }
        },
        AlgorithmParameters::Other(_) | _ => Err(IncomingTokenError::UnsupportedKeyType {
            uri: uri.to_owned(),
            index,
        }),
    }
}

fn select_key<'keys>(
    keys: &'keys [SigningJwk],
    requested_kid: Option<&str>,
) -> Option<&'keys SigningJwk> {
    match requested_kid {
        Some(kid) if !kid.is_empty() => keys.iter().find(|key| key.kid.as_deref() == Some(kid)),
        Some(_) | None => keys.first(),
    }
}

fn read_kid(token: &str) -> Result<Option<String>, IncomingTokenError> {
    let segment = token.split('.').next().unwrap_or_default();
    let bytes = URL_SAFE_NO_PAD
        .decode(segment.trim_end_matches('='))
        .map_err(|source| IncomingTokenError::HeaderBase64 { source })?;
    let header: UnverifiedHeader = serde_json::from_slice(&bytes)
        .map_err(|source| IncomingTokenError::HeaderJson { source })?;
    Ok(header.kid)
}
