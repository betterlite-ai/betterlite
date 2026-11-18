use sha2::{Digest as _, Sha256};

// proxy/utils.py hash_token: sha256 of the UTF-8 bytes, lowercase hex.
#[must_use]
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[path = "hashing/tests.rs"]
mod tests;
