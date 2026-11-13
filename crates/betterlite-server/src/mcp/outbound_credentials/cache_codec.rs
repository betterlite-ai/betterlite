mod tests;

use std::collections::BTreeMap;

use thiserror::Error;

use super::{BOUND_PREFIX, OAuthToken};

pub type Encrypt = Box<dyn Fn(&str) -> String + Send + Sync>;
pub type Decrypt = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct OAuthTokenCacheCodec {
    pub encrypt: Encrypt,
    pub decrypt: Decrypt,
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("the bound token payload could not be serialized")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
}

impl OAuthTokenCacheCodec {
    pub fn encode(&self, token: &OAuthToken) -> Result<String, CodecError> {
        let Some(proof) = &token.identity_binding_proof else {
            return Ok((self.encrypt)(&token.access_token));
        };
        let payload = BTreeMap::from([
            ("access_token", token.access_token.as_str()),
            ("identity_binding_proof", proof.as_str()),
        ]);
        let serialized =
            serde_json::to_string(&payload).map_err(|source| CodecError::Serialize { source })?;
        Ok((self.encrypt)(&format!("{BOUND_PREFIX}{serialized}")))
    }

    #[must_use]
    pub fn decode(&self, blob: &str) -> Option<OAuthToken> {
        let plaintext = (self.decrypt)(blob)?;
        if plaintext.is_empty() {
            return None;
        }
        let Some(bound) = plaintext.strip_prefix(BOUND_PREFIX) else {
            return Some(OAuthToken {
                access_token: plaintext,
                expires_at: None,
                refresh_token: None,
                scopes: Vec::new(),
                identity_binding_proof: None,
            });
        };
        let payload: BTreeMap<String, String> = serde_json::from_str(bound).ok()?;
        let bearer = payload.get("access_token")?;
        let proof = payload.get("identity_binding_proof")?;
        if bearer.is_empty() || proof.is_empty() {
            return None;
        }
        Some(OAuthToken {
            access_token: bearer.clone(),
            expires_at: None,
            refresh_token: None,
            scopes: Vec::new(),
            identity_binding_proof: Some(proof.clone()),
        })
    }
}
