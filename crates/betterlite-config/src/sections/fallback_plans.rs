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
        let (ordinary, ordinary_path) =
            configured(router.fallbacks, litellm.fallbacks, "fallbacks");
        let mut fallbacks = one_group_entries(ordinary, &ordinary_path)?;
        let defaults = router.default_fallbacks.or(litellm.default_fallbacks);
        if let Some(targets) = defaults {
            fallbacks.push(FallbackChainEntry::Group(IndexMap::from([(
                String::from("*"),
                targets,
            )])));
        }
        let (policy, policy_path) = configured(
            router.content_policy_fallbacks,
            litellm.content_policy_fallbacks,
            "content_policy_fallbacks",
        );
        Ok(Settings {
            fallbacks,
            context_window_fallbacks: configured(
                router.context_window_fallbacks,
                litellm.context_window_fallbacks,
                "context_window_fallbacks",
            )
            .0,
            content_policy_fallbacks: one_group_entries(policy, &policy_path)?,
            max_fallbacks: max_fallbacks(router.max_fallbacks.or(litellm.max_fallbacks), lookup)?,
        })
    }
}

fn configured(
    router: Option<Vec<FallbackChainEntry>>,
    litellm: Option<Vec<FallbackChainEntry>>,
    name: &str,
) -> (Vec<FallbackChainEntry>, String) {
    match (router, litellm) {
        (Some(entries), _) => (entries, format!("router_settings.{name}")),
        (None, Some(entries)) => (entries, format!("litellm_settings.{name}")),
        (None, None) => (Vec::new(), format!("router_settings.{name}")),
    }
}

// LiteLLM validates the ordinary and content-policy lists only, and context-window chains keep bare and multi-key entries
fn one_group_entries(
    entries: Vec<FallbackChainEntry>,
    path: &str,
) -> Result<Vec<FallbackChainEntry>, ConfigError> {
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
