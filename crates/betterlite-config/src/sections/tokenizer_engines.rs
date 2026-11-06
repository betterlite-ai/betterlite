use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use std::path::PathBuf;
mod tests;
#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default, rename = "litellm_settings")]
    settings: Option<RawSettings>,
}
#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
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
        Ok(Settings {
            disable_hf_tokenizer_download: self
                .settings
                .unwrap_or_default()
                .disable_hf_tokenizer_download
                .unwrap_or(false),
            custom_tiktoken_cache_dir: lookup
                .lookup("CUSTOM_TIKTOKEN_CACHE_DIR")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            max_concurrent_counts: max_concurrent_counts(
                &lookup
                    .lookup("TOKEN_COUNTER_MAX_CONCURRENT_COUNTS")
                    .unwrap_or_default(),
            ),
        })
    }
}
// LiteLLM's get_env_int_in_range: a missing or unparsable value takes the default silently, an out-of-range one warns first
fn max_concurrent_counts(configured: &str) -> usize {
    let Ok(value) = configured.trim().parse::<i64>() else {
        return 4;
    };
    if (1..=256).contains(&value) {
        return usize::try_from(value).unwrap_or(4);
    }
    tracing::warn!(
        "TOKEN_COUNTER_MAX_CONCURRENT_COUNTS={value} is outside the supported range [1, 256]. Falling back to 4."
    );
    4
}
