use secrecy::SecretString;

use super::{ProviderEnvironment, ProviderInfo, resolve_api_base, resolve_api_key};

pub const DEFAULT_API_BASE: &str = "http://127.0.0.1:8080/v1";
const PLACEHOLDER_KEY: &str = "fake-api-key";

#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &ProviderEnvironment,
) -> ProviderInfo {
    ProviderInfo {
        api_base: resolve_api_base(api_base, environment)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
        api_key: resolve_api_key(api_key, environment, PLACEHOLDER_KEY),
    }
}
