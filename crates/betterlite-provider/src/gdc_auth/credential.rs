use std::sync::Arc;

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1_SIGNING as P256_SIGNING, ECDSA_P384_SHA384_ASN1_SIGNING as P384_SIGNING,
    EcdsaKeyPair, EcdsaSigningAlgorithm,
};
use jsonwebtoken::errors::Error as JsonWebTokenError;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use pkcs8::der::asn1::{AnyRef, OctetStringRef};
use pkcs8::der::{Decode as _, Encode as _, Tag, Tagged as _};
use pkcs8::{AlgorithmIdentifierRef, ObjectIdentifier, PrivateKeyInfoRef};
use sec1::{EcParameters, EcPrivateKey};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::gdc_auth::key::GdchDescriptor;

pub const JWT_LIFETIME_SECONDS: i64 = 3_600;
const MAX_PEM_BYTES: usize = 0x4000;
const EC_PUBLIC_KEY_OID: &str = "1.2.840.10045.2.1";
#[rustfmt::skip]
const P256_CURVE_OID_DER: [u8; 10] = [0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
#[rustfmt::skip]
const P256_CURVE_OID_CONTENT: [u8; 8] = [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
#[rustfmt::skip]
const P384_CURVE_OID_DER: [u8; 7] = [0x06, 0x05, 0x2B, 0x81, 0x04, 0x00, 0x22];
#[rustfmt::skip]
const P384_CURVE_OID_CONTENT: [u8; 5] = [0x2B, 0x81, 0x04, 0x00, 0x22];

#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve { P256, P384 }
#[rustfmt::skip]
impl Curve {
    #[must_use]
    pub const fn algorithm(self) -> Algorithm { match self { Self::P256 => Algorithm::ES256, Self::P384 => Algorithm::ES384 } }
    #[must_use]
    pub const fn signing(self) -> &'static EcdsaSigningAlgorithm { match self { Self::P256 => &P256_SIGNING, Self::P384 => &P384_SIGNING } }
    #[must_use]
    pub const fn named_curve_der(self) -> &'static [u8] { match self { Self::P256 => &P256_CURVE_OID_DER, Self::P384 => &P384_CURVE_OID_DER } }
}

#[rustfmt::skip]
#[derive(Debug)]
pub struct GdchServiceAccount { pub descriptor: GdchDescriptor, pub signer: GdchSigner }
#[rustfmt::skip]
#[derive(Debug)]
pub enum GdchSigner { Ec { key: EncodingKey, curve: Curve }, Fixture(SecretString) }
#[rustfmt::skip]
#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the GDC Gemini private key is not a PEM document")] Pem,
    #[error("the GDC Gemini private key is not an elliptic curve key")] NotEc,
    #[error("the GDC Gemini private key uses a curve the token exchange does not support")] Curve,
    #[error("the GDC Gemini private key is not usable")] Key,
    #[error("the GDC Gemini service account token could not be signed")] Sign { #[source] source: JsonWebTokenError },
    #[error("a fixture credential cannot sign a token")] Fixture,
}

pub trait GdchCredentialFactory: Send + Sync {
    fn build(&self, descriptor: GdchDescriptor)
    -> Result<Arc<GdchServiceAccount>, CredentialError>;
}

pub struct PemCredentials;

#[rustfmt::skip]
impl GdchCredentialFactory for PemCredentials {
    fn build(&self, descriptor: GdchDescriptor) -> Result<Arc<GdchServiceAccount>, CredentialError> { GdchServiceAccount::load(descriptor).map(Arc::new) }
}

// The oracle projection uses fixture credentials so it never loads a private key, refreshes or opens a path
#[rustfmt::skip]
#[derive(Debug, Clone)]
pub struct FixtureCredentials { pub token: SecretString }
#[rustfmt::skip]
impl GdchCredentialFactory for FixtureCredentials {
    fn build(&self, descriptor: GdchDescriptor) -> Result<Arc<GdchServiceAccount>, CredentialError> {
        Ok(Arc::new(GdchServiceAccount { descriptor, signer: GdchSigner::Fixture(self.token.clone()) }))
    }
}

#[rustfmt::skip]
#[derive(Serialize)]
struct GdchClaims<'value> { iss: &'value str, sub: &'value str, aud: &'value str, iat: i64, exp: i64 }

impl GdchServiceAccount {
    #[rustfmt::skip]
    pub fn load(descriptor: GdchDescriptor) -> Result<Self, CredentialError> {
        let signer = parse_private_key(descriptor.private_key.expose_secret())?;
        Ok(Self {
            descriptor,
            signer,
        })
    }
    #[rustfmt::skip]
    pub fn sign(&self, now: i64) -> Result<String, CredentialError> {
        let GdchSigner::Ec { key, curve } = &self.signer else {
            return Err(CredentialError::Fixture);
        };
        let subject = format!("system:serviceaccount:{}:{}", self.descriptor.project, self.descriptor.name);
        let claims = GdchClaims { iss: &subject, sub: &subject, aud: &self.descriptor.token_uri, iat: now, exp: now.saturating_add(JWT_LIFETIME_SECONDS) };
        let mut header = Header::new(curve.algorithm());
        header.kid = Some(self.descriptor.private_key_id.clone());
        jsonwebtoken::encode(&header, &claims, key).map_err(|source| CredentialError::Sign { source })
    }
}

#[rustfmt::skip]
fn parse_private_key(pem_text: &str) -> Result<GdchSigner, CredentialError> {
    if pem_text.len() > MAX_PEM_BYTES {
        return Err(CredentialError::Pem);
    }
    let block = pem::parse(pem_text.as_bytes()).map_err(|_error| CredentialError::Pem)?;
    let tag = block.tag().to_owned();
    let contents = Zeroizing::new(block.into_contents());
    match tag.as_str() {
        "PRIVATE KEY" => pkcs8_signer(&contents),
        "EC PRIVATE KEY" => sec1_signer(&contents),
        _ => Err(CredentialError::NotEc),
    }
}

#[rustfmt::skip]
fn pkcs8_signer(der: &[u8]) -> Result<GdchSigner, CredentialError> {
    let info = PrivateKeyInfoRef::from_der(der).map_err(|_error| CredentialError::Pem)?;
    if info.algorithm.oid != ObjectIdentifier::new(EC_PUBLIC_KEY_OID).map_err(|_error| CredentialError::Pem)? {
        return Err(CredentialError::NotEc);
    }
    let parameters = info.algorithm.parameters.ok_or(CredentialError::NotEc)?;
    if parameters.tag() != Tag::ObjectIdentifier {
        return Err(CredentialError::NotEc);
    }
    build_signer(der, curve_for(parameters.value())?)
}

#[rustfmt::skip]
fn sec1_signer(der: &[u8]) -> Result<GdchSigner, CredentialError> {
    let key = EcPrivateKey::from_der(der).map_err(|_error| CredentialError::Pem)?;
    let Some(EcParameters::NamedCurve(named_curve)) = key.parameters else {
        return Err(CredentialError::NotEc);
    };
    let curve = curve_for(named_curve.as_bytes())?;
    let parameters = AnyRef::try_from(curve.named_curve_der()).map_err(|_error| CredentialError::Pem)?;
    let identifier = AlgorithmIdentifierRef { oid: ObjectIdentifier::new(EC_PUBLIC_KEY_OID).map_err(|_error| CredentialError::Pem)?, parameters: Some(parameters) };
    let private_key = OctetStringRef::new(der).map_err(|_error| CredentialError::Pem)?;
    let pkcs8 = Zeroizing::new(PrivateKeyInfoRef::new(identifier, private_key).to_der().map_err(|_error| CredentialError::Pem)?);
    build_signer(&pkcs8, curve)
}

#[rustfmt::skip]
fn curve_for(curve: &[u8]) -> Result<Curve, CredentialError> {
    if curve == P256_CURVE_OID_CONTENT.as_slice() {
        Ok(Curve::P256)
    } else if curve == P384_CURVE_OID_CONTENT.as_slice() {
        Ok(Curve::P384)
    } else {
        Err(CredentialError::Curve)
    }
}

#[rustfmt::skip]
fn build_signer(pkcs8_der: &[u8], curve: Curve) -> Result<GdchSigner, CredentialError> {
    EcdsaKeyPair::from_pkcs8(curve.signing(), pkcs8_der).map_err(|_error| CredentialError::Key)?;
    let key = EncodingKey::from_ec_der(pkcs8_der);
    Ok(GdchSigner::Ec { key, curve })
}
