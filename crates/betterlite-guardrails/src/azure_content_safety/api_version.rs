pub const DEFAULT_API_VERSION: &str = "2024-09-01";

// Older releases stored the Javelin API version in the same field, and LiteLLM upgrades it.
pub const LEGACY_JAVELIN_VERSION: &str = "v1";

#[must_use]
pub fn resolve_api_version(configured: Option<&str>) -> &str {
    match configured {
        Some(version) if version != LEGACY_JAVELIN_VERSION => version,
        None | Some(_) => DEFAULT_API_VERSION,
    }
}
