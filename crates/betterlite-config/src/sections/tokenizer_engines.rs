use std::path::PathBuf;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const DEFAULT_MAX_CONCURRENT_COUNTS: i64 = 4;
const MIN_MAX_CONCURRENT_COUNTS: i64 = 1;
const MAX_MAX_CONCURRENT_COUNTS: i64 = 256;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    #[serde(rename = "litellm_settings")]
    settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    #[serde(default)]
    disable_hf_tokenizer_download: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub disable_hf_tokenizer_download: bool,
    pub custom_tiktoken_cache_dir: Option<PathBuf>,
    pub max_concurrent_counts: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            disable_hf_tokenizer_download: false,
            custom_tiktoken_cache_dir: None,
            max_concurrent_counts: 4,
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured_cache = lookup
            .lookup("CUSTOM_TIKTOKEN_CACHE_DIR")
            .filter(|value| !value.is_empty());
        let configured_counts = lookup
            .lookup("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS")
            .unwrap_or_default();
        Ok(Settings {
            disable_hf_tokenizer_download: self
                .settings
                .disable_hf_tokenizer_download
                .unwrap_or(false),
            custom_tiktoken_cache_dir: configured_cache.map(PathBuf::from),
            max_concurrent_counts: max_concurrent_counts(&configured_counts),
        })
    }
}

// LiteLLM's get_env_int_in_range: a missing or unparsable value takes the default silently, an out-of-range one warns first
fn max_concurrent_counts(configured: &str) -> usize {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return 4;
    }
    let Ok(value) = trimmed.parse::<i64>() else {
        return 4;
    };
    if (MIN_MAX_CONCURRENT_COUNTS..=MAX_MAX_CONCURRENT_COUNTS).contains(&value) {
        return usize::try_from(value).unwrap_or(4);
    }
    tracing::warn!(
        "TOKEN_COUNTER_MAX_CONCURRENT_COUNTS={value} is outside the supported range [1, 256]. Falling back to {DEFAULT_MAX_CONCURRENT_COUNTS}."
    );
    usize::try_from(DEFAULT_MAX_CONCURRENT_COUNTS).unwrap_or(4)
}
