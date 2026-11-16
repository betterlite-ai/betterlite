pub mod error;
pub mod identity;
pub mod params;
pub mod reply;
pub mod request;
mod tests;

use secrecy::ExposeSecret as _;

use crate::openai_base_config::Headers;
use error::DashscopeImageError;
use identity::{ImageIdentity, resolve_image_key};

// the image endpoint's plain DashScope default is the international host, unlike its chat default
pub const DEFAULT_API_BASE: &str =
    "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";
pub const CHAT_COMPATIBLE_MODE_PATH: &str = "/compatible-mode/v1";
pub const AUTHORIZATION_HEADER: &str = "Authorization";
pub const CONTENT_TYPE_HEADER: &str = "Content-Type";
pub const JSON_CONTENT_TYPE: &str = "application/json";
pub const MISSING_KEY_MESSAGE: &str = "DASHSCOPE_API_KEY is not set";

#[derive(Debug, Clone)]
pub struct DashscopeImageConfig {
    pub identity: ImageIdentity,
}

impl DashscopeImageConfig {
    // LiteLLM adds its two headers to the caller's dict, so its values win over the caller's
    pub fn validate_environment(
        &self,
        api_key: Option<&str>,
        headers: &Headers,
    ) -> Result<Headers, DashscopeImageError> {
        let key = resolve_image_key(&self.identity, api_key)?;
        let mut resolved = headers.clone();
        resolved.insert(
            AUTHORIZATION_HEADER.to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        resolved.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(resolved)
    }

    // a chat-shaped base is dropped, then an empty environment base, then the default answer as is
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>) -> String {
        let api_base = api_base.filter(|base| {
            !base.is_empty()
                && !base
                    .trim_end_matches('/')
                    .ends_with(CHAT_COMPATIBLE_MODE_PATH)
        });
        let env_image_base = self
            .identity
            .env_image_base
            .as_deref()
            .filter(|base| !base.is_empty());
        api_base
            .or(env_image_base)
            .map_or_else(|| self.identity.default_image_base.clone(), str::to_owned)
    }
}
