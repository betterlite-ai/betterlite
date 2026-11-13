use betterlite_openai::api::image_contracts::records::ImageHeaders;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::fal::images::error::FalImageError;

pub const DEFAULT_BASE_URL: &str = "https://fal.run";

#[derive(Debug, Clone)]
pub struct FalImageSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl Default for FalImageSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            api_base: DEFAULT_BASE_URL.to_owned(),
        }
    }
}

impl FalImageSettings {
    #[must_use]
    pub fn complete_url(&self, api_base: Option<&str>, endpoint: &str) -> String {
        let base = api_base
            .filter(|explicit| !explicit.is_empty())
            .unwrap_or(&self.api_base)
            .trim_end_matches('/');
        if endpoint.is_empty() {
            base.to_owned()
        } else {
            format!("{base}/{endpoint}")
        }
    }

    pub fn authorize(
        &self,
        headers: &ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, FalImageError> {
        let key = [api_key, self.api_key.as_ref()]
            .into_iter()
            .flatten()
            .find(|secret| !secret.expose_secret().is_empty())
            .ok_or(FalImageError::MissingApiKey)?;
        let mut authorized = headers.clone();
        authorized.insert(
            "Authorization".to_owned(),
            format!("Key {}", key.expose_secret()),
        );
        Ok(authorized)
    }
}
