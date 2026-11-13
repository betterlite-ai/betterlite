use crate::loader::EnvironmentLookup;

mod tests;

pub const GENERIC_CLIENT_STATE_ENV: &str = "GENERIC_CLIENT_STATE";
pub const GENERIC_CLIENT_USE_PKCE_ENV: &str = "GENERIC_CLIENT_USE_PKCE";
pub const PKCE_STRICT_CACHE_MISS_ENV: &str = "PKCE_STRICT_CACHE_MISS";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PkceSettings {
    pub use_pkce: bool,
    pub strict_cache_miss: bool,
    pub generic_client_state: Option<String>,
}

pub fn load_pkce_settings<L>(lookup: &L) -> PkceSettings
where
    L: EnvironmentLookup,
{
    // LiteLLM lowercases each flag and compares it with exactly "true"; any other text leaves it off
    PkceSettings {
        use_pkce: lookup
            .lookup(GENERIC_CLIENT_USE_PKCE_ENV)
            .is_some_and(|value| value.to_lowercase() == "true"),
        strict_cache_miss: lookup
            .lookup(PKCE_STRICT_CACHE_MISS_ENV)
            .is_some_and(|value| value.to_lowercase() == "true"),
        // Python truthiness makes an empty configured state fall through to the generated one
        generic_client_state: lookup
            .lookup(GENERIC_CLIENT_STATE_ENV)
            .filter(|value| !value.is_empty()),
    }
}
