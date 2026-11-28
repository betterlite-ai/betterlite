use std::fmt::Display;
use std::str::FromStr as _;

use aws_lc_rs::signature::{ECDSA_P521_SHA512_FIXED, ParsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use jsonwebtoken::{Algorithm, DecodingKey, crypto};
use x509_parser::public_key::PublicKey as CertificateKey;

use super::algorithms::{Family, JwtAlgorithm};
use super::errors::VerificationError;
use crate::jwt_claims::decode::decode_segment;

// _decode_jwt_with_public_key accepts a JWKS dictionary or a PEM X.509 certificate string and nothing else.
pub enum PublicKey<'key> {
    Jwk(&'key Json),
    Certificate(&'key str),
}

impl<'key> PublicKey<'key> {
    #[must_use]
    pub fn from_resolved(value: &'key Json) -> Option<Self> {
        match value {
            Json::Object(_) => Some(Self::Jwk(value)),
            Json::Text(text) => Some(Self::Certificate(text)),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
        }
    }
}

// The signature covers the token's signing input; the algorithm was already limited to the ten allowed names.
pub fn verify_signature(
    public_key: &PublicKey<'_>,
    algorithm: JwtAlgorithm,
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    match public_key {
        PublicKey::Jwk(jwk) => verify_with_jwk(jwk, algorithm, signing_input, signature),
        PublicKey::Certificate(pem) => {
            verify_with_certificate(pem, algorithm, signing_input, signature)
        }
    }
}

// _get_jwk_from_public_key projects the key to kty/kid/n/e/x/y/crv before PyJWK builds it; nothing else is read.
fn verify_with_jwk(
    jwk: &Json,
    algorithm: JwtAlgorithm,
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    let Json::Object(fields) = jwk else {
        return Err(not_a_key_object());
    };
    let key_type = required_text(fields, "kty")?;
    match (algorithm.family(), key_type.as_str()) {
        (Family::Rsa, "RSA") => {
            let modulus = required_text(fields, "n")?;
            let exponent = required_text(fields, "e")?;
            let key = DecodingKey::from_rsa_components(&modulus, &exponent)
                .map_err(|error| unusable_key(&error))?;
            verify_with_library(&key, algorithm, signing_input, signature)
        }
        (Family::Ec, "EC") => {
            let curve = required_text(fields, "crv")?;
            if Some(curve.as_str()) != algorithm.curve() {
                return Err(incompatible_key());
            }
            let abscissa = required_text(fields, "x")?;
            let ordinate = required_text(fields, "y")?;
            if Algorithm::from_str(algorithm.wire_name()).is_err() {
                return verify_p521_point(&abscissa, &ordinate, signing_input, signature);
            }
            let key = DecodingKey::from_ec_components(&abscissa, &ordinate)
                .map_err(|error| unusable_key(&error))?;
            verify_with_library(&key, algorithm, signing_input, signature)
        }
        (Family::Ed, "OKP") => {
            let curve = required_text(fields, "crv")?;
            if curve != "Ed25519" {
                return Err(incompatible_key());
            }
            let key_text = required_text(fields, "x")?;
            let key =
                DecodingKey::from_ed_components(&key_text).map_err(|error| unusable_key(&error))?;
            verify_with_library(&key, algorithm, signing_input, signature)
        }
        (Family::Rsa | Family::Ec | Family::Ed, _) => Err(incompatible_key()),
    }
}

fn required_text(fields: &IndexMap<String, Json>, name: &str) -> Result<String, VerificationError> {
    match fields.get(name) {
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => Err(incompatible_key()),
    }
}

fn verify_with_library(
    key: &DecodingKey,
    algorithm: JwtAlgorithm,
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    let Ok(library) = Algorithm::from_str(algorithm.wire_name()) else {
        return Err(incompatible_key());
    };
    match crypto::verify(signature, signing_input, key, library) {
        Ok(true) => Ok(()),
        Ok(false) => Err(invalid_signature()),
        Err(error) => Err(unusable_key(&error)),
    }
}

// ES512 has no jsonwebtoken verifier, so the JWK's affine coordinates become a SEC1 point for aws-lc-rs.
fn verify_p521_point(
    abscissa: &str,
    ordinate: &str,
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    let mut point = Vec::with_capacity(133);
    point.push(4_u8);
    for coordinate in [abscissa, ordinate] {
        point.extend_from_slice(
            &URL_SAFE_NO_PAD
                .decode(coordinate)
                .map_err(|error| unusable_key(&error))?,
        );
    }
    verify_with_p521(&point, signing_input, signature)
}

fn verify_with_p521(
    point: &[u8],
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    let key = ParsedPublicKey::new(&ECDSA_P521_SHA512_FIXED, point)
        .map_err(|error| unusable_key(&error))?;
    let signature_bytes =
        decode_segment(signature.as_bytes()).map_err(|error| unusable_key(&error))?;
    key.verify_sig(signing_input, &signature_bytes)
        .map_err(|_error| invalid_signature())
}

// x509.load_pem_x509_certificate: a CERTIFICATE PEM block, then the SPKI inside it.
fn verify_with_certificate(
    pem_text: &str,
    algorithm: JwtAlgorithm,
    signing_input: &[u8],
    signature: &str,
) -> Result<(), VerificationError> {
    let block = pem::parse(pem_text.as_bytes()).map_err(|error| unusable_key(&error))?;
    if block.tag() != "CERTIFICATE" {
        return Err(VerificationError::Validation {
            detail: "the configured public key is not a PEM X.509 certificate".to_owned(),
        });
    }
    let Ok((_, certificate)) = x509_parser::parse_x509_certificate(block.contents()) else {
        return Err(unusable_certificate());
    };
    let public_key = certificate
        .public_key()
        .parsed()
        .map_err(|error| unusable_key(&error))?;
    if let CertificateKey::RSA(rsa) = &public_key {
        let key =
            DecodingKey::from_rsa_raw_components(unsigned(rsa.modulus), unsigned(rsa.exponent));
        return verify_with_library(&key, algorithm, signing_input, signature);
    }
    if let CertificateKey::EC(point) = &public_key {
        if Algorithm::from_str(algorithm.wire_name()).is_ok() {
            let key = DecodingKey::from_ec_der(point.data());
            return verify_with_library(&key, algorithm, signing_input, signature);
        }
        return verify_with_p521(point.data(), signing_input, signature);
    }
    if let CertificateKey::Unknown(bytes) = &public_key {
        if algorithm.family() != Family::Ed || bytes.len() != 32 {
            return Err(incompatible_key());
        }
        let key = DecodingKey::from_ed_der(bytes);
        return verify_with_library(&key, algorithm, signing_input, signature);
    }
    Err(incompatible_key())
}

// x509-parser keeps DER INTEGER bytes as written, and a leading zero only pads the unsigned value.
pub(super) const fn unsigned(bytes: &[u8]) -> &[u8] {
    match bytes {
        [0_u8, rest @ ..] if !rest.is_empty() => rest,
        _ => bytes,
    }
}

fn invalid_signature() -> VerificationError {
    VerificationError::Validation {
        detail: "Signature verification failed".to_owned(),
    }
}

fn incompatible_key() -> VerificationError {
    VerificationError::Validation {
        detail: "the configured key is not compatible with the token algorithm".to_owned(),
    }
}

fn not_a_key_object() -> VerificationError {
    VerificationError::Validation {
        detail: "the configured public key is not a JWK object".to_owned(),
    }
}

fn unusable_certificate() -> VerificationError {
    VerificationError::Validation {
        detail: "the configured public key is not a PEM X.509 certificate".to_owned(),
    }
}

fn unusable_key(error: &dyn Display) -> VerificationError {
    VerificationError::Validation {
        detail: format!("the configured key is not usable: {error}"),
    }
}
