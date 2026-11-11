use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::resolution::inferred::inferred_provider;
use strum::VariantArray as _;

// The caller owns the resolver's tables, the same split the guardrails crate uses for its provider names.
pub struct ProviderNames<'context> {
    pub catalog: &'context ModelCatalog,
    pub known_model_names: &'context BTreeSet<String>,
    pub custom_providers: &'context BTreeSet<String>,
    pub environment: &'context BTreeMap<String, String>,
    pub registry: &'context JSONProviderRegistry,
}

// providers.json registers these slugs, but litellm.types.utils.LlmProviders does not, so provider_list lacks them.
const REGISTRY_ONLY_PROVIDERS: [ProviderId; 8] = [
    ProviderId::Abliteration,
    ProviderId::Aihubmix,
    ProviderId::Crusoe,
    ProviderId::Empiriolabs,
    ProviderId::Gmi,
    ProviderId::Llamagate,
    ProviderId::Sarvam,
    ProviderId::Veniceai,
];

impl ProviderNames<'_> {
    // litellm.provider_list is list(LlmProviders) plus each custom_provider_map provider (custom_llm_setup).
    fn slugs(&self) -> impl Iterator<Item = &str> {
        let listed = ProviderId::VARIANTS
            .iter()
            .filter(|provider| !REGISTRY_ONLY_PROVIDERS.contains(provider))
            .map(|provider| <&'static str>::from(*provider));
        listed.chain(self.custom_providers.iter().map(String::as_str))
    }

    fn inferred(&self, model: &str) -> Option<String> {
        let context = ResolutionContext {
            catalog: self.catalog,
            known_model_names: self.known_model_names,
            custom_providers: self.custom_providers,
            environment: self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        };
        inferred_provider(model, &context).map(|provider| provider.slug().to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainMatch<'plan> {
    pub targets: Vec<&'plan FallbackTarget>,
    pub generic: bool,
}

#[must_use]
pub fn get_fallback_model_group<'plan>(
    chain: &'plan [FallbackChainEntry],
    model_group: &str,
    names: &ProviderNames<'_>,
) -> Option<ChainMatch<'plan>> {
    let prefixed = prefixed_model_group(model_group, &chain_keys(chain), names);
    let mut generic: Option<&'plan [FallbackTarget]> = None;
    let mut stripped: Option<&'plan [FallbackTarget]> = None;
    let mut resolved: Option<ChainMatch<'plan>> = None;
    for entry in chain {
        match entry {
            FallbackChainEntry::Group(group) => {
                let Some((name, targets)) = group.iter().next() else {
                    continue;
                };
                if name == model_group {
                    resolved = Some(ChainMatch {
                        targets: targets.iter().collect(),
                        generic: false,
                    });
                    break;
                }
                let matched = prefixed.as_deref() == Some(name.as_str())
                    || stripped_model_group(model_group, name, names);
                if matched {
                    stripped = Some(targets.as_slice());
                }
                if name == "*" {
                    generic = Some(targets.as_slice());
                }
            }
            FallbackChainEntry::Target(target) => {
                resolved = Some(ChainMatch {
                    targets: vec![target],
                    generic: false,
                });
            }
        }
    }
    select(resolved, stripped, generic)
}

fn chain_keys(chain: &[FallbackChainEntry]) -> Vec<&str> {
    chain
        .iter()
        .filter_map(|entry| match entry {
            FallbackChainEntry::Group(group) => group.keys().next().map(String::as_str),
            FallbackChainEntry::Target(_) => None,
        })
        .collect()
}

fn select<'plan>(
    resolved: Option<ChainMatch<'plan>>,
    stripped: Option<&'plan [FallbackTarget]>,
    generic: Option<&'plan [FallbackTarget]>,
) -> Option<ChainMatch<'plan>> {
    if resolved.is_some() {
        return resolved;
    }
    if let Some(targets) = stripped {
        return Some(ChainMatch {
            targets: targets.iter().collect(),
            generic: false,
        });
    }
    generic.map(|targets| ChainMatch {
        targets: targets.iter().collect(),
        generic: true,
    })
}

#[must_use]
pub fn resolve_for_lookup_groups<'plan>(
    chain: &'plan [FallbackChainEntry],
    lookup_groups: &[&str],
    names: &ProviderNames<'_>,
) -> Option<ChainMatch<'plan>> {
    let mut generic: Option<ChainMatch<'plan>> = None;
    for group in lookup_groups {
        let Some(found) = get_fallback_model_group(chain, group, names) else {
            continue;
        };
        if !found.generic {
            return Some(found);
        }
        generic.get_or_insert(found);
    }
    generic
}

// LiteLLM strips every occurrence with str.replace, so a group naming its provider twice loses both
fn stripped_model_group(model_group: &str, fallback_key: &str, names: &ProviderNames<'_>) -> bool {
    for slug in names.slugs() {
        let Some(rest) = model_group.strip_prefix(slug) else {
            continue;
        };
        if !rest.starts_with('/') {
            continue;
        }
        let prefix = format!("{slug}/");
        if model_group.replace(&prefix, "") == fallback_key {
            return true;
        }
    }
    false
}

fn prefixed_model_group(
    model_group: &str,
    fallback_keys: &[&str],
    names: &ProviderNames<'_>,
) -> Option<String> {
    if model_group.contains('/') {
        return None;
    }
    let listed = fallback_keys.iter().any(|key| {
        key.strip_suffix(model_group)
            .is_some_and(|prefix| prefix.ends_with('/'))
    });
    if !listed {
        return None;
    }
    let provider = names.inferred(model_group)?;
    Some(format!("{provider}/{model_group}"))
}
