use std::str::Utf8Error;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("no LITELLM_SALT_KEY or master key is set to encrypt with")]
    MissingKey,
    #[error("Set LITELLM_SALT_KEY or a master key to mint bearer tokens")]
    BearerSecret,
    #[error("the encoded secret is {length} bytes, over the {limit} byte limit")]
    TooLarge { length: usize, limit: usize },
    #[error("the encoded secret is not valid base64")]
    Base64,
    #[error("the encoded secret is not valid base64")]
    Decode {
        #[source]
        source: base64::DecodeError,
    },
    #[error("the encoded secret is not valid base64")]
    DecodeTail {
        #[source]
        source: data_encoding::DecodeError,
    },
    #[error("the secret is too short to hold its nonce")]
    Truncated,
    #[error("the ciphertext failed authentication under this key")]
    Seal {
        #[source]
        source: aes_gcm::Error,
    },
    #[error("the ciphertext failed authentication under this key")]
    LegacySeal {
        #[source]
        source: crypto_secretbox::Error,
    },
    #[error("the decrypted secret is not UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("could not draw a random nonce")]
    Random {
        #[source]
        source: getrandom::Error,
    },
    #[error("could not encode the secret map")]
    MapEncode {
        #[source]
        source: serde_json::Error,
    },
    #[error("Cannot decode encrypted MCP {field}; check LITELLM_SALT_KEY")]
    MapDecode { field: String },
}
