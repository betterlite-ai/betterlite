use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

const TTL_PATH: &str = "litellm_settings.anthropic_prompt_caching_ttl";
const TTL_ENVIRONMENT_VARIABLE: &str = "LITELLM_ANTHROPIC_PROMPT_CACHING_TTL";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    anthropic_prompt_caching_ttl: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheTtl {
    FiveMinutes,
    OneHour,
}

impl CacheTtl {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FiveMinutes => "5m",
            Self::OneHour => "1h",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub ttl: Option<CacheTtl>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let configured = match self.litellm_settings.anthropic_prompt_caching_ttl {
            Some(text) => Some(resolve_string(text, TTL_PATH, lookup)?),
            // LiteLLM reads the variable once at import and the config value overrides it, so the variable only supplies the default.
            None => lookup.lookup(TTL_ENVIRONMENT_VARIABLE),
        };
        Ok(Settings {
            ttl: configured.as_deref().and_then(ttl_from_setting),
        })
    }
}

// LiteLLM's Literal holds only these two spellings; any other value leaves Anthropic's 5-minute ephemeral default.
fn ttl_from_setting(setting: &str) -> Option<CacheTtl> {
    match setting {
        "5m" => Some(CacheTtl::FiveMinutes),
        "1h" => Some(CacheTtl::OneHour),
        _ => None,
    }
}
