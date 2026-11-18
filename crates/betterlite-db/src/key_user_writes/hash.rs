use sha2::{Digest as _, Sha256};

const KEY_PREFIX: &str = "sk-";

// LiteLLM hashes the UTF-8 bytes of any stored key, with or without a prefix, and keeps the lowercase hex
#[must_use]
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

// Only a plaintext key carries the sk- prefix; any other string is already a stored identifier
#[must_use]
pub fn hash_token_if_needed(token: &str) -> String {
    if token.starts_with(KEY_PREFIX) {
        return hash_token(token);
    }
    token.to_owned()
}
