use indexmap::IndexMap;
use serde::Deserialize;

use self::chains::{FallbackChainEntry, FallbackTarget};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

pub mod chains;
mod tests;

const DEFAULT_MAX_FALLBACKS: i64 = 5;
const MAX_FALLBACKS_VARIABLE: &str = "ROUTER_MAX_FALLBACKS";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    router_settings: RawRouter,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawRouter {
    fallbacks: Option<Vec<FallbackChainEntry>>,
    context_window_fallbacks: Option<Vec<FallbackChainEntry>>,
    content_policy_fallbacks: Option<Vec<FallbackChainEntry>>,
    default_fallbacks: Option<Vec<FallbackTarget>>,
    max_fallbacks: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    fallbacks: Option<Vec<FallbackChainEntry>>,
    context_window_fallbacks: Option<Vec<FallbackChainEntry>>,
    content_policy_fallbacks: Option<Vec<FallbackChainEntry>>,
    default_fallbacks: Option<Vec<FallbackTarget>>,
    max_fallbacks: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub fallbacks: Vec<FallbackChainEntry>,
    pub context_window_fallbacks: Vec<FallbackChainEntry>,
    pub content_policy_fallbacks: Vec<FallbackChainEntry>,
    pub max_fallbacks: i64,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let router = self.router_settings;
        let litellm = self.litellm_settings;
        let mut fallbacks = chain(router.fallbacks, litellm.fallbacks, "fallbacks")?;
        let defaults = router.default_fallbacks.or(litellm.default_fallbacks);
        if let Some(targets) = defaults {
            fallbacks.push(FallbackChainEntry::Group(IndexMap::from([(
                String::from("*"),
                targets,
            )])));
        }
        Ok(Settings {
            fallbacks,
            context_window_fallbacks: chain(
                router.context_window_fallbacks,
                litellm.context_window_fallbacks,
                "context_window_fallbacks",
            )?,
            content_policy_fallbacks: chain(
                router.content_policy_fallbacks,
                litellm.content_policy_fallbacks,
                "content_policy_fallbacks",
            )?,
            max_fallbacks: max_fallbacks(router.max_fallbacks.or(litellm.max_fallbacks), lookup)?,
        })
    }
}

// LiteLLM accepts a chain entry only as a dictionary keyed by exactly one model group
fn chain(
    router: Option<Vec<FallbackChainEntry>>,
    litellm: Option<Vec<FallbackChainEntry>>,
    name: &str,
) -> Result<Vec<FallbackChainEntry>, ConfigError> {
    let (entries, path) = match (router, litellm) {
        (Some(entries), _) => (entries, format!("router_settings.{name}")),
        (None, Some(entries)) => (entries, format!("litellm_settings.{name}")),
        (None, None) => return Ok(Vec::new()),
    };
    for (index, entry) in entries.iter().enumerate() {
        if !matches!(entry, FallbackChainEntry::Group(group) if group.len() == 1) {
            return Err(ConfigError::InvalidScalar {
                path: format!("{path}[{index}]"),
                value: false,
            });
        }
    }
    Ok(entries)
}

// ROUTER_MAX_FALLBACKS is the declaration default a router setting overrides, and an empty one counts as unset
fn max_fallbacks<L>(setting: Option<i64>, lookup: &L) -> Result<i64, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(value) = setting {
        return Ok(value);
    }
    let Some(environment) = lookup.lookup(MAX_FALLBACKS_VARIABLE) else {
        return Ok(DEFAULT_MAX_FALLBACKS);
    };
    Ok(
        betterlite_env::number::<i64>(MAX_FALLBACKS_VARIABLE, &environment)?
            .unwrap_or(DEFAULT_MAX_FALLBACKS),
    )
}
