use regex::Regex;
use secrecy::SecretString;

// Python's \s also matches U+001C to U+001F, which Rust's does not, so the classes spell them out
const AWS_BEARER_CREDENTIAL: &str = r"Credential=Bearer[\s\x{1c}-\x{1f}]+([^/\s\x{1c}-\x{1f},]+)";
const AWS_CREDENTIAL: &str = r"Credential=([^/\s\x{1c}-\x{1f},]+)";

mod tests;

#[must_use]
pub fn get_bearer_token(api_key: &str) -> Option<SecretString> {
    if api_key.starts_with("Bearer ") {
        return Some(SecretString::from(api_key.replace("Bearer ", "")));
    }
    if api_key.starts_with("Basic ") {
        return Some(SecretString::from(api_key.replace("Basic ", "")));
    }
    if api_key.starts_with("bearer ") {
        return Some(SecretString::from(api_key.replace("bearer ", "")));
    }
    if api_key.starts_with("AWS4-HMAC-SHA256") {
        return captured_token(AWS_BEARER_CREDENTIAL, api_key)
            .or_else(|| captured_token(AWS_CREDENTIAL, api_key))
            .map(SecretString::from);
    }
    None
}

// both patterns are constants, so a failed compile cannot happen and None means no credential field
fn captured_token(pattern: &str, api_key: &str) -> Option<String> {
    let regex = Regex::new(pattern).ok()?;
    regex
        .captures(api_key)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str().to_owned())
}
