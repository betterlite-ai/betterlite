use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};

// the provider names a model group can carry, and the inference that names the provider of a group that carries none
pub struct ProviderNames<'names> {
    pub slugs: &'names [String],
    pub inferred: &'names dyn Fn(&str) -> Option<String>,
}

// the chain one lookup group resolved to, and whether it is the "*" entry rather than a group-specific one
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainMatch<'plan> {
    pub targets: Vec<&'plan FallbackTarget>,
    pub generic: bool,
}

// The chain a lookup name resolves to: exact match, stripped or provider-prefixed match, then the generic "*" entry
#[must_use]
pub fn get_fallback_model_group<'plan>(
    chain: &'plan [FallbackChainEntry],
    model_group: &str,
    names: &ProviderNames<'_>,
) -> Option<ChainMatch<'plan>> {
    let prefixed = prefixed_model_group(model_group, &chain_keys(chain), names.inferred);
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
                    || stripped_model_group(model_group, name, names.slugs);
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

// An exact match or a bare entry wins, then the last stripped match, then the generic entry
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

// First lookup group with a specifically keyed chain wins; the generic "*" chain applies only after every group missed
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
fn stripped_model_group(model_group: &str, fallback_key: &str, slugs: &[String]) -> bool {
    for slug in slugs {
        let Some(rest) = model_group.strip_prefix(slug.as_str()) else {
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
    inferred: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    if model_group.contains('/') {
        return None;
    }
    let named = fallback_keys.iter().any(|key| {
        key.strip_suffix(model_group)
            .is_some_and(|prefix| prefix.ends_with('/'))
    });
    if !named {
        return None;
    }
    let provider = inferred(model_group)?;
    Some(format!("{provider}/{model_group}"))
}
