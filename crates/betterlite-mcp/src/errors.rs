use aws_lc_rs::error::{KeyRejected, Unspecified};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionKeyError {
    #[error("the session signing key must be at least 32 characters")]
    ShortSigningKey,
    #[error("every kid must be non-empty")]
    EmptyKid,
    #[error("every kid must be unique across the current and previous keys; duplicated: {kids}")]
    DuplicateKid { kids: String },
    #[error("the key is not a PEM document")]
    Pem {
        #[source]
        source: pem::PemError,
    },
    #[error("private_key_pem must be an unencrypted RSA private key of 2048 to 8192 bits")]
    PrivateKey {
        #[source]
        source: KeyRejected,
    },
    #[error("public_key_pem must be an RSA public key of 2048 to 8192 bits")]
    PublicKey {
        #[source]
        source: KeyRejected,
    },
    #[error("the RSA public key cannot be encoded")]
    PublicEncoding {
        #[source]
        source: Unspecified,
    },
}

#[derive(Debug, Error)]
pub enum SessionMintError {
    #[error("session token is too large: {size_bytes} bytes, maximum {max_bytes}")]
    TooLarge { size_bytes: usize, max_bytes: usize },
    #[error("a session principal needs a non-empty user_id and client_id")]
    EmptyIdentity,
    #[error("the session token expiry is out of range")]
    ExpiryOutOfRange,
    #[error("the jti random source failed")]
    Random {
        #[source]
        source: getrandom::Error,
    },
    #[error("the session claims cannot be serialized")]
    Claims {
        #[source]
        source: serde_json::Error,
    },
    #[error("the RSA signature failed")]
    Signature {
        #[source]
        source: Unspecified,
    },
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq, Serialize)]
#[serde(tag = "tag", rename_all = "snake_case")]
pub enum SessionOpenError {
    #[error("candidate is not a session token")]
    NotASessionToken,
    #[error("session token signature is invalid")]
    #[serde(rename = "session_bad_signature")]
    BadSignature,
    #[error("session token is expired")]
    #[serde(rename = "session_expired")]
    Expired,
    #[error("session token is malformed")]
    #[serde(rename = "session_malformed")]
    Malformed,
}
