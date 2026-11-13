pub const PROTECTED_HEADERS: [&str; 8] = [
    "authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "ocp-apim-subscription-key",
    "host",
    "content-length",
    "accept-encoding",
];

pub const PROTECTED_HEADER_PREFIXES: [&str; 1] = ["x-amz-"];

#[must_use]
pub fn is_protected(name: &str) -> bool {
    let lowered = name.to_lowercase();
    if PROTECTED_HEADERS.contains(&lowered.as_str()) {
        return true;
    }
    PROTECTED_HEADER_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
}

mod tests;
