use betterlite_models::key_digest::hashing::hash_token;
use betterlite_models::key_digest::is_valid_sha256_hash;

const CLI_SESSION_VALUE_PREFIX: &str = "cli-session-";
const HASHED_JWT_PREFIX: &str = "hashed-jwt-";
const NON_SECRET_KEY_ALIASES: [&str; 3] = [
    "litellm_proxy_master_key",
    "litellm-internal-health-check",
    "litellm-cli",
];

// _redact_logged_api_key: drop one leading Bearer-space, keep a non-secret value only when the caller proved it trusted.
#[must_use]
pub fn redact_logged_api_key(
    value: Option<&str>,
    already_redacted: bool,
    key_alias: Option<&str>,
) -> Option<String> {
    let value = value?;
    if value.is_empty() {
        return None;
    }
    let stripped = strip_bearer(value);
    if stripped.is_empty() {
        return None;
    }
    if already_redacted && is_non_secret_key_value(stripped, key_alias) {
        return Some(stripped.to_owned());
    }
    Some(hash_token(stripped))
}

#[must_use]
pub fn is_non_secret_key_value(value: &str, key_alias: Option<&str>) -> bool {
    NON_SECRET_KEY_ALIASES.contains(&value)
        || is_valid_sha256_hash(value)
        || is_hashed_jwt(value)
        || is_cli_session_alias(value, key_alias)
}

// re.sub(r"(?i)^bearer ", ""): exactly one case-insensitive ASCII prefix.
fn strip_bearer(value: &str) -> &str {
    match value.get(..7) {
        Some(prefix) if prefix.eq_ignore_ascii_case("bearer ") => &value[7..],
        Some(_) | None => value,
    }
}

// _HASHED_JWT_RE.fullmatch(r"hashed-jwt-[a-fA-F0-9]{64}")
fn is_hashed_jwt(value: &str) -> bool {
    value
        .strip_prefix(HASHED_JWT_PREFIX)
        .is_some_and(is_valid_sha256_hash)
}

fn is_cli_session_alias(value: &str, key_alias: Option<&str>) -> bool {
    let Some(alias) = key_alias else {
        return false;
    };
    value.starts_with(CLI_SESSION_VALUE_PREFIX) && value == alias
}
