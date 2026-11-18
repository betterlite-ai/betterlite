use sha2::{Digest as _, Sha256};

// proxy/utils.py hash_token: sha256 of the UTF-8 bytes, lowercase hex.
#[must_use]
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[path = "hashing/tests.rs"]
mod tests;
