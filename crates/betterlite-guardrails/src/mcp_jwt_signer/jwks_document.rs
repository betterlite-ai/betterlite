use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;

use crate::mcp_jwt_signer::signing_key::SigningKey;

mod tests;

pub const SIGNING_ALGORITHM: &str = "RS256";
const KEY_TYPE: &str = "RSA";
const KEY_USE: &str = "sig";
const PERSISTENT_MAX_AGE_SECONDS: u64 = 3600;
const GENERATED_MAX_AGE_SECONDS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JwksDocument {
    pub keys: Vec<JwkKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JwkKey {
    pub kty: &'static str,
    pub alg: &'static str,
    #[serde(rename = "use")]
    pub usage: &'static str,
    pub kid: String,
    #[serde(rename = "n")]
    pub modulus: String,
    #[serde(rename = "e")]
    pub exponent: String,
}

#[must_use]
pub fn get_jwks(key: &SigningKey) -> JwksDocument {
    JwksDocument {
        keys: vec![JwkKey {
            kty: KEY_TYPE,
            alg: SIGNING_ALGORITHM,
            usage: KEY_USE,
            kid: key.kid.clone(),
            modulus: URL_SAFE_NO_PAD.encode(&key.modulus),
            exponent: URL_SAFE_NO_PAD.encode(&key.exponent),
        }],
    }
}

#[must_use]
pub const fn jwks_max_age(key: &SigningKey) -> u64 {
    if key.persistent {
        PERSISTENT_MAX_AGE_SECONDS
    } else {
        GENERATED_MAX_AGE_SECONDS
    }
}
