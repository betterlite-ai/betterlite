// LiteLLM's fullmatch(r"[a-fA-F0-9]{64}") accepts exactly 64 ASCII hex digits.
#[must_use]
pub fn is_valid_sha256_hash(value: &str) -> bool {
    if value.len() != 64 {
        return false;
    }
    value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[path = "key_digest/tests.rs"]
mod tests;
