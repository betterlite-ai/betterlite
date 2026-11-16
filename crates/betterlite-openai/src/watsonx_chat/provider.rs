use secrecy::SecretString;

use crate::hosted_vllm::chat::ProviderInfo;
use crate::local_servers::{ProviderEnvironment, resolve_api_base, resolve_api_key};

// the environment holds HOSTED_VLLM_API_BASE and HOSTED_VLLM_API_KEY, and vllm needs no key, so the last fallback is ""
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
    environment: &ProviderEnvironment,
) -> ProviderInfo {
    ProviderInfo {
        api_base: resolve_api_base(api_base, environment),
        api_key: resolve_api_key(api_key, environment, ""),
    }
}
