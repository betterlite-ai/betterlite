use secrecy::{ExposeSecret as _, SecretString};

use super::ProxyOptions;
use super::error::ProxyError;
use crate::openai_base_config::Headers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEndpoint {
    Generations,
    Edits,
}

impl ImageEndpoint {
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Generations => "/images/generations",
            Self::Edits => "/images/edits",
        }
    }
}

// validate_environment replaces any Authorization the caller sent, even when no key resolves
#[must_use]
pub fn image_headers(
    options: &ProxyOptions,
    headers: &Headers,
    api_key: Option<&SecretString>,
) -> Headers {
    let resolved = options.resolve_api_key(api_key);
    let authorization = resolved.as_ref().map_or_else(
        || String::from("Bearer None"),
        |key| format!("Bearer {}", key.expose_secret()),
    );
    let mut environment = headers.clone();
    environment.insert(String::from("Authorization"), authorization);
    environment
}

pub fn image_url(
    options: &ProxyOptions,
    api_base: Option<&str>,
    endpoint: ImageEndpoint,
) -> Result<String, ProxyError> {
    let Some(base) = options.resolve_api_base(api_base) else {
        return Err(ProxyError::MissingApiBase);
    };
    Ok(format!("{}{}", base.trim_end_matches('/'), endpoint.path()))
}
