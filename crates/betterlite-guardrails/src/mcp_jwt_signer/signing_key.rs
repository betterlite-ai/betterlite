use std::fs;
use std::io;
use std::path::PathBuf;

use aws_lc_rs::encoding::AsDer as _;
use aws_lc_rs::error::{KeyRejected, Unspecified};
use aws_lc_rs::rsa::{KeyPair, KeySize};
use aws_lc_rs::signature::KeyPair as _;
use betterlite_config::loader::EnvironmentLookup;
use jsonwebtoken::errors::Error as JsonWebTokenError;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::mcp_token_claims::Claims;

mod tests;

pub const SIGNING_KEY_ENV: &str = "MCP_JWT_SIGNING_KEY";
const FILE_PREFIX: &str = "file://";
const KID_HEX_CHARS: usize = 16;

#[derive(Debug)]
pub struct SigningKey {
    pub(crate) encoding: EncodingKey,
    pub(crate) kid: String,
    pub(crate) modulus: Vec<u8>,
    pub(crate) exponent: Vec<u8>,
    pub(crate) persistent: bool,
}

#[derive(Debug, Error)]
pub enum SigningKeyError {
    #[error("MCP_JWT_SIGNING_KEY points at {path} which cannot be read: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("MCP_JWT_SIGNING_KEY does not hold a PEM document: {source}")]
    Pem {
        #[source]
        source: pem::PemError,
    },
    #[error("MCP_JWT_SIGNING_KEY does not hold a PKCS#8 private key: {source}")]
    Pkcs8 {
        #[source]
        source: pkcs8::Error,
    },
    #[error("MCP_JWT_SIGNING_KEY does not hold an RSA private key: {source}")]
    PrivateKey {
        #[source]
        source: KeyRejected,
    },
    #[error("cannot generate an RSA key pair: {source}")]
    Generate {
        #[source]
        source: Unspecified,
    },
    #[error("cannot encode the RSA key: {source}")]
    Encode {
        #[source]
        source: Unspecified,
    },
    #[error("cannot sign the outbound token: {source}")]
    Sign {
        #[source]
        source: JsonWebTokenError,
    },
}

impl SigningKey {
    pub fn load(env: &dyn EnvironmentLookup) -> Result<Self, SigningKeyError> {
        let Some(material) = env
            .lookup(SIGNING_KEY_ENV)
            .filter(|material| !material.is_empty())
        else {
            let key = Self::generate()?;
            tracing::info!(
                "MCPJWTSigner: generated an RSA-2048 key pair (set {SIGNING_KEY_ENV} to use your own key)"
            );
            return Ok(key);
        };
        let key = Self::from_pem(&read_material(&material)?)?;
        tracing::info!("MCPJWTSigner: loaded the RSA key from {SIGNING_KEY_ENV}");
        Ok(key)
    }

    pub fn sign(&self, claims: &Claims) -> Result<SecretString, SigningKeyError> {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(self.kid.clone());
        let token = jsonwebtoken::encode(&header, claims, &self.encoding)
            .map_err(|source| SigningKeyError::Sign { source })?;
        Ok(SecretString::from(token))
    }

    fn generate() -> Result<Self, SigningKeyError> {
        let key_pair = KeyPair::generate(KeySize::Rsa2048)
            .map_err(|source| SigningKeyError::Generate { source })?;
        let document = key_pair
            .as_der()
            .map_err(|source| SigningKeyError::Encode { source })?;
        let private_der = pkcs8_private_key(document.as_ref())?;
        Self::from_key_pair(&key_pair, &private_der, false)
    }

    fn from_pem(material: &[u8]) -> Result<Self, SigningKeyError> {
        let document = pem::parse(material).map_err(|source| SigningKeyError::Pem { source })?;
        let private_der = if document.tag() == "RSA PRIVATE KEY" {
            document.contents().to_vec()
        } else {
            pkcs8_private_key(document.contents())?
        };
        let key_pair = KeyPair::from_der(&private_der)
            .map_err(|source| SigningKeyError::PrivateKey { source })?;
        Self::from_key_pair(&key_pair, &private_der, true)
    }

    fn from_key_pair(
        key_pair: &KeyPair,
        private_der: &[u8],
        persistent: bool,
    ) -> Result<Self, SigningKeyError> {
        let public = key_pair.public_key();
        let spki = public
            .as_der()
            .map_err(|source| SigningKeyError::Encode { source })?;
        let digest = hex::encode(Sha256::digest(spki.as_ref()));
        let kid: String = digest.chars().take(KID_HEX_CHARS).collect();
        Ok(Self {
            encoding: EncodingKey::from_rsa_der(private_der),
            kid,
            modulus: public.modulus().big_endian_without_leading_zero().to_vec(),
            exponent: public.exponent().big_endian_without_leading_zero().to_vec(),
            persistent,
        })
    }
}

fn read_material(material: &str) -> Result<Vec<u8>, SigningKeyError> {
    let Some(path) = material.strip_prefix(FILE_PREFIX) else {
        return Ok(material.as_bytes().to_vec());
    };
    fs::read(path).map_err(|source| SigningKeyError::ReadFile {
        path: PathBuf::from(path),
        source,
    })
}

fn pkcs8_private_key(document: &[u8]) -> Result<Vec<u8>, SigningKeyError> {
    let info = pkcs8::PrivateKeyInfoRef::try_from(document)
        .map_err(|source| SigningKeyError::Pkcs8 { source })?;
    Ok(info.private_key.as_bytes().to_vec())
}
