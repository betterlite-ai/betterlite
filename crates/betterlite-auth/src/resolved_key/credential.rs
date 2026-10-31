use sha2::{Digest as _, Sha256};

mod tests;

const BEARER_PREFIX_LEN: usize = 7;

#[must_use]
pub fn hash_token(value: &str) -> String {
    let hashed_token = Sha256::digest(value.as_bytes());
    hex::encode(hashed_token)
}

#[must_use]
pub fn is_jwt(token: Option<&str>) -> bool {
    let Some(value) = token else {
        return false;
    };
    value.split('.').count() == 3
}

fn strip_bearer_prefix(value: &str) -> &str {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(BEARER_PREFIX_LEN).collect();
    if prefix.chars().count() == BEARER_PREFIX_LEN && prefix.eq_ignore_ascii_case("bearer ") {
        chars.as_str()
    } else {
        value
    }
}

#[must_use]
pub fn safe_hash_api_key(api_key: &str) -> String {
    let normalized = strip_bearer_prefix(api_key);
    if normalized.starts_with("sk-") {
        return hash_token(normalized);
    }
    if is_jwt(Some(normalized)) {
        return format!("hashed-jwt-{}", hash_token(normalized));
    }
    normalized.to_owned()
}
