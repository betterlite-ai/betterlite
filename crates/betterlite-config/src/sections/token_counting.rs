use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use std::num::NonZeroUsize;

mod tests;

const MAX_EXACT_ENV: &str = "TOKEN_COUNTER_MAX_EXACT_CHARS";
const CHUNK_ENV: &str = "TIKTOKEN_ENCODE_CHUNK_SIZE_CHARS";
const DEFAULT_MAX_EXACT_CHARS: usize = 4_000_000;
const DEFAULT_CHUNK_CHARS: usize = 1024;
const MAX_EXACT_LIMIT: usize = 1_000_000_000;
// LiteLLM's TIKTOKEN_ENCODE_MAX_CHUNK_SIZE_CHARS keeps a misconfigured chunk size from restoring quadratic BPE
const CHUNK_LIMIT: usize = 4096;

#[derive(Debug, Default, serde::Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawSettings {
    disable_token_counter: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub disable_token_counter: bool,
    pub max_exact_chars: NonZeroUsize,
    pub chunk_chars: NonZeroUsize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            disable_token_counter: false,
            max_exact_chars: NonZeroUsize::new(DEFAULT_MAX_EXACT_CHARS)
                .unwrap_or(NonZeroUsize::MIN),
            chunk_chars: NonZeroUsize::new(DEFAULT_CHUNK_CHARS).unwrap_or(NonZeroUsize::MIN),
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
            disable_token_counter: self.litellm_settings.disable_token_counter.unwrap_or(false),
            max_exact_chars: bounded(
                lookup,
                MAX_EXACT_ENV,
                DEFAULT_MAX_EXACT_CHARS,
                MAX_EXACT_LIMIT,
            ),
            chunk_chars: bounded(lookup, CHUNK_ENV, DEFAULT_CHUNK_CHARS, CHUNK_LIMIT),
        })
    }
}

// LiteLLM's get_env_int_in_range: an unset or unparsable value takes the default silently, an out-of-range one warns first
fn bounded<L>(lookup: &L, name: &str, default: usize, maximum: usize) -> NonZeroUsize
where
    L: EnvironmentLookup,
{
    let fallback = NonZeroUsize::new(default).unwrap_or(NonZeroUsize::MIN);
    let Some(text) = lookup.lookup(name) else {
        return fallback;
    };
    let Ok(value) = text.trim().parse::<i64>() else {
        return fallback;
    };
    let parsed = usize::try_from(value)
        .ok()
        .filter(|found| *found <= maximum);
    let Some(found) = parsed.and_then(NonZeroUsize::new) else {
        tracing::warn!(
            "{name}={value} is outside the supported range [1, {maximum}]. Falling back to {}.",
            fallback.get()
        );
        return fallback;
    };
    found
}
