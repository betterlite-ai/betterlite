#[must_use]
pub fn is_valid_sha256_hash(value: &str) -> bool {
    // LiteLLM accepts only a SHA-256 digest there, so a raw key never lands in the hash field
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

mod tests;
