pub const LOCALHOST_URL_PATTERNS: [&str; 4] = ["localhost", "127.0.0.1", "0.0.0.0", "[::1]"];

#[must_use]
pub fn is_localhost_or_internal_url(url: Option<&str>) -> bool {
    url.is_some_and(|candidate| {
        let lowered = candidate.to_lowercase();
        LOCALHOST_URL_PATTERNS
            .iter()
            .any(|pattern| lowered.contains(pattern))
    })
}
