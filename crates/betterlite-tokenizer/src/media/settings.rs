use serde::Deserialize;

use crate::image::settings::Settings as ImageSettings;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageCounting {
    #[default]
    Exact,
    DefaultCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub image: ImageSettings,
    // LiteLLM returns this count when a content block raises, discarding the blocks counted before it
    pub fallback_token_count: Option<u64>,
    pub image_counting: ImageCounting,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            image: ImageSettings::default(),
            fallback_token_count: None,
            image_counting: ImageCounting::Exact,
        }
    }
}
