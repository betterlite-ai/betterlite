use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_secret;
use crate::sections::Section;
use secrecy::SecretString;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLitellmSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawLitellmSettings {
    api_key: Option<SecretString>,
}

// One host's environment values: the key alias and the two base aliases in LiteLLM's fallback order
#[derive(Debug, Clone, Default)]
pub struct HostEnvironment {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub fallback_api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    // litellm.api_key: the last value every host's key chain falls back to
    pub global_api_key: Option<SecretString>,
    pub deepseek: HostEnvironment,
    pub tencent: HostEnvironment,
    pub minimax: HostEnvironment,
}

impl Settings {
    #[must_use]
    pub fn resolve_environment<L>(lookup: &L) -> Self
    where
        L: EnvironmentLookup,
    {
        Self {
            global_api_key: None,
            deepseek: environment(
                lookup,
                "DEEPSEEK_API_KEY",
                "DEEPSEEK_ANTHROPIC_API_BASE",
                Some("DEEPSEEK_API_BASE"),
            ),
            tencent: environment(
                lookup,
                "TENCENT_API_KEY",
                "TENCENT_ANTHROPIC_API_BASE",
                Some("TENCENT_API_BASE"),
            ),
            minimax: environment(lookup, "MINIMAX_API_KEY", "MINIMAX_API_BASE", None),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let mut settings = Settings::resolve_environment(lookup);
        settings.global_api_key = resolve_secret(
            self.litellm_settings.api_key,
            "litellm_settings.api_key",
            lookup,
        )?;
        Ok(settings)
    }
}

// LiteLLM reads every alias behind Python's `or`, so an empty environment value falls through as unset
fn environment<L>(lookup: &L, key: &str, base: &str, fallback: Option<&str>) -> HostEnvironment
where
    L: EnvironmentLookup,
{
    HostEnvironment {
        api_key: lookup
            .lookup(key)
            .filter(|value| !value.is_empty())
            .map(SecretString::from),
        api_base: lookup.lookup(base).filter(|value| !value.is_empty()),
        fallback_api_base: fallback
            .and_then(|name| lookup.lookup(name))
            .filter(|value| !value.is_empty()),
    }
}
