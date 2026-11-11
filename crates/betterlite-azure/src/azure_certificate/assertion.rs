use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::CertificateError;
use super::material::CertificateMaterial;
use crate::azure_client_secret::ClientSecretError;

const LIFETIME_SECONDS: i64 = 600;
// msal wraps the assertion in an AutoRefresher that keeps it for the lifetime minus one minute
const REUSE_SECONDS: i64 = 540;

// the jti source is injected so a fixture can pin it
pub trait IdSource: Send + Sync {
    fn next_id(&self) -> String;
}

pub struct RandomIds;

impl IdSource for RandomIds {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[derive(Serialize)]
struct Claims<'text> {
    aud: &'text str,
    iss: &'text str,
    sub: &'text str,
    exp: i64,
    iat: i64,
    jti: &'text str,
}

pub struct Assertions {
    pub material: CertificateMaterial,
    pub client_id: String,
    pub send_chain: bool,
    pub ids: Arc<dyn IdSource>,
    pub reusable: Mutex<Option<(SecretString, i64)>>,
}

impl Assertions {
    pub fn assertion(&self, audience: &str, now: i64) -> Result<SecretString, CertificateError> {
        let mut reusable = self
            .reusable
            .lock()
            .map_err(|_error| ClientSecretError::ProviderCachePoisoned)?;
        if let Some((assertion, expires_at)) = reusable.as_ref()
            && *expires_at > now
        {
            return Ok(assertion.clone());
        }
        let signed = self.sign(audience, now)?;
        *reusable = Some((signed.clone(), now.saturating_add(REUSE_SECONDS)));
        drop(reusable);
        Ok(signed)
    }

    fn header(&self) -> Header {
        let mut header = Header::new(Algorithm::RS256);
        header.x5t = Some(URL_SAFE.encode(self.material.thumbprint));
        if self.send_chain {
            header.x5c = Some(
                self.material
                    .chain
                    .iter()
                    .map(|der| STANDARD.encode(der))
                    .collect(),
            );
        }
        header
    }

    fn sign(&self, audience: &str, now: i64) -> Result<SecretString, CertificateError> {
        let jti = self.ids.next_id();
        let claims = Claims {
            aud: audience,
            iss: &self.client_id,
            sub: &self.client_id,
            exp: now.saturating_add(LIFETIME_SECONDS),
            iat: now,
            jti: &jti,
        };
        let key = EncodingKey::from_rsa_der(self.material.key.expose_secret());
        jsonwebtoken::encode(&self.header(), &claims, &key)
            .map(SecretString::from)
            .map_err(|source| CertificateError::Sign { source })
    }
}
