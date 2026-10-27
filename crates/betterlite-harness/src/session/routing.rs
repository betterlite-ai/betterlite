use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::context::GatewayTarget;

pub const LITELLM_PROXY_PREFIX: &str = "litellm_proxy/";

#[derive(Debug, Default)]
pub struct ProxySettings {
    pub use_litellm_proxy: bool,
    // supplied by the configuration boundary from LITELLM_PROXY_API_BASE and LITELLM_PROXY_API_KEY
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug, Error)]
pub enum RouteError {
    #[error(
        "litellm_proxy/ models need the gateway URL: pass api_base= or set LITELLM_PROXY_API_BASE"
    )]
    MissingBase,
    #[error(
        "litellm_proxy/ models need a gateway virtual key: pass api_key= or set LITELLM_PROXY_API_KEY"
    )]
    MissingKey,
}

pub fn resolve_model_route(
    model: Option<&str>,
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    proxy: &ProxySettings,
) -> Result<(Option<String>, Option<GatewayTarget>), RouteError> {
    let group = model.and_then(|value| value.strip_prefix(LITELLM_PROXY_PREFIX));
    if group.is_none() && !proxy.use_litellm_proxy {
        return Ok((model.map(str::to_owned), None));
    }
    let base = api_base
        .filter(|value| !value.is_empty())
        .or(proxy.api_base.as_deref())
        .unwrap_or_default()
        .trim();
    let key = api_key
        .filter(|value| !value.expose_secret().is_empty())
        .or(proxy.api_key.as_ref())
        .map_or("", ExposeSecret::expose_secret)
        .trim();
    if base.is_empty() {
        return Err(RouteError::MissingBase);
    }
    if key.is_empty() {
        return Err(RouteError::MissingKey);
    }
    Ok((
        group.or(model).map(str::to_owned),
        Some(GatewayTarget {
            api_base: base.trim_end_matches('/').to_owned(),
            api_key: SecretString::from(key),
        }),
    ))
}
