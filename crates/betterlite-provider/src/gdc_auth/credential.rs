use std::sync::Arc;

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1_SIGNING as P256_SIGNING, ECDSA_P384_SHA384_ASN1_SIGNING as P384_SIGNING,
    EcdsaKeyPair, EcdsaSigningAlgorithm,
};
use jsonwebtoken::errors::Error as JsonWebTokenError;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroizing;

use crate::gdc_auth::key::GdchDescriptor;

pub const JWT_LIFETIME_SECONDS: i64 = 3_600;
const MAX_PEM_BYTES: usize = 0x4000;

#[rustfmt::skip]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve { P256, P384 }
#[rustfmt::skip]
impl Curve {
    #[must_use]
    pub const fn algorithm(self) -> Algorithm { match self { Self::P256 => Algorithm::ES256, Self::P384 => Algorithm::ES384 } }
    #[must_use]
    pub const fn signing(self) -> &'static EcdsaSigningAlgorithm { match self { Self::P256 => &P256_SIGNING, Self::P384 => &P384_SIGNING } }
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
    #[error("the GDC Gemini private key is not a usable P-256 or P-384 key")] Key,
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
    fn build(&self, descriptor: GdchDescriptor) -> Result<Arc<GdchServiceAccount>, CredentialError> {
        Ok(Arc::new(GdchServiceAccount { signer: parse_private_key(descriptor.private_key.expose_secret())?, descriptor }))
    }
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
    if !["PRIVATE KEY", "EC PRIVATE KEY"].contains(&block.tag()) {
        return Err(CredentialError::NotEc);
    }
    let contents = Zeroizing::new(block.into_contents());
    for curve in [Curve::P256, Curve::P384] {
        if let Ok(pair) = EcdsaKeyPair::from_private_key_der(curve.signing(), &contents) {
            let pkcs8 = pair.to_pkcs8v1().map_err(|_error| CredentialError::Key)?;
            return Ok(GdchSigner::Ec { key: EncodingKey::from_ec_der(pkcs8.as_ref()), curve });
        }
    }
    Err(CredentialError::Key)
}
