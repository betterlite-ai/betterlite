use secrecy::{ExposeSecret as _, SecretString};

use crate::openai_base_config::{CatalogCapabilities, OpenAiConfig};

mod tests;

pub const DEFAULT_API_BASE: &str = "https://api.minimax.io/v1";
pub const PROVIDER: &str = "minimax";
// LiteLLM's MiniMax config returns messages and tools unchanged, so B503's cache_control strip is skipped here
pub const KEEPS_CACHE_CONTROL: bool = true;

// LiteLLM chains the explicit base, the MINIMAX_API_BASE secret, then the default, skipping an empty one at each step
#[must_use]
pub fn resolve_base(api_base: Option<&str>, env_base: Option<&str>) -> String {
    api_base
        .filter(|value| !value.is_empty())
        .or(env_base.filter(|value| !value.is_empty()))
        .unwrap_or(DEFAULT_API_BASE)
        .to_owned()
}

// get_api_key chains the argument, the MINIMAX_API_KEY secret, then the global key the caller passed
#[must_use]
pub fn provider_info(
    api_base: Option<&str>,
    env_base: Option<&str>,
    api_key: Option<&SecretString>,
    env_key: Option<&SecretString>,
    global_key: Option<&SecretString>,
) -> (String, Option<SecretString>) {
    let key = [api_key, env_key, global_key]
        .into_iter()
        .flatten()
        .find(|key| !key.expose_secret().is_empty())
        .cloned();
    (resolve_base(api_base, env_base), key)
}

// a base ending in /chat/completions is complete, one ending in /v1 gains the suffix, one ending in / gains v1 first
#[must_use]
pub fn complete_url(api_base: Option<&str>, env_base: Option<&str>) -> String {
    let base = resolve_base(api_base, env_base);
    if base.ends_with("/chat/completions") {
        return base;
    }
    if base.ends_with("/v1") {
        return format!("{base}/chat/completions");
    }
    if base.ends_with('/') {
        return format!("{base}v1/chat/completions");
    }
    format!("{base}/v1/chat/completions")
}

// LiteLLM appends reasoning_split always and thinking only when the capability lookup says the model reasons
#[must_use]
pub fn supported_params(
    model: &str,
    config: &OpenAiConfig,
    catalog: &CatalogCapabilities,
    supports_reasoning: bool,
) -> Vec<&'static str> {
    let mut params = config.supported_params(model, catalog);
    params.push("reasoning_split");
    if supports_reasoning {
        params.push("thinking");
    }
    params
}
