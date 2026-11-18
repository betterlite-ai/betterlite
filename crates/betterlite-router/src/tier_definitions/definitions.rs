use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::tier_definitions as section;
use indexmap::IndexMap;

use crate::complexity_scoring::defaults::{self, Defaults};
use crate::complexity_scoring::tier::ComplexityTier;
use crate::tier_definitions::checks::{ClassifierFallback, ClassifierKind, TierChecks};
use crate::tier_definitions::error::{DefinitionConflict, TierDefinitionsError};
use crate::tier_definitions::model::{
    MAX_TIER_DEFINITIONS, TierDefinition, TierModelEntry, TierPool,
};

const MIN_TIER_DEFINITIONS: usize = 2;
const MAX_TIER_NAME_CHARS: usize = 64;
const MAX_TIER_DESCRIPTION_CHARS: usize = 500;

pub(super) fn resolve_pools(
    input: &section::Config,
) -> Result<BTreeMap<String, TierPool>, TierDefinitionsError> {
    let Some(configured) = &input.tier_models else {
        return pinned_pools();
    };
    let mut pools = BTreeMap::new();
    for (tier, pool) in configured {
        pools.insert(tier.clone(), normalize_pool(tier, pool)?);
    }
    Ok(pools)
}

// tiers is absent, so LiteLLM's pinned default map applies
fn pinned_pools() -> Result<BTreeMap<String, TierPool>, TierDefinitionsError> {
    let defaults: Defaults = serde_json::from_str(defaults::SOURCE)
        .map_err(|source| TierDefinitionsError::Defaults { source })?;
    let mut pools = BTreeMap::new();
    for (tier, model) in defaults.tier_models {
        let entry = TierModelEntry {
            model_name: model,
            params: IndexMap::new(),
        };
        pools.insert(tier, TierPool::Single(entry));
    }
    Ok(pools)
}

fn normalize_pool(tier: &str, pool: &section::TierPool) -> Result<TierPool, TierDefinitionsError> {
    match pool {
        section::TierPool::Single(entry) => Ok(TierPool::Single(normalize_entry(entry))),
        section::TierPool::Pool(entries) => {
            let normalized: Vec<TierModelEntry> = entries.iter().map(normalize_entry).collect();
            reject_duplicate_names(tier, &normalized)?;
            Ok(TierPool::Pool(normalized))
        }
    }
}

// LiteLLM rejects a pool that names the same model twice, even with different params
fn reject_duplicate_names(
    tier: &str,
    entries: &[TierModelEntry],
) -> Result<(), TierDefinitionsError> {
    let distinct: BTreeSet<&str> = entries
        .iter()
        .map(|entry| entry.model_name.as_str())
        .collect();
    if distinct.len() == entries.len() {
        return Ok(());
    }
    Err(TierDefinitionsError::DuplicateModelName {
        tier: tier.to_owned(),
    })
}

fn normalize_entry(entry: &section::TierEntry) -> TierModelEntry {
    match entry {
        section::TierEntry::Model(model) => TierModelEntry {
            model_name: model.clone(),
            params: IndexMap::new(),
        },
        section::TierEntry::Detailed(detailed) => TierModelEntry {
            model_name: detailed.model_name.clone(),
            params: detailed.litellm_params.clone(),
        },
    }
}

pub(super) fn normalize_definition(
    raw: &section::TierDefinition,
) -> Result<TierDefinition, TierDefinitionsError> {
    let name = raw.name.trim().to_owned();
    let description = raw
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    if name.is_empty() {
        return Err(TierDefinitionsError::EmptyName);
    }
    if name.chars().count() > MAX_TIER_NAME_CHARS {
        return Err(TierDefinitionsError::NameTooLong {
            name,
            max: MAX_TIER_NAME_CHARS,
        });
    }
    if description
        .as_deref()
        .is_some_and(|text| text.chars().count() > MAX_TIER_DESCRIPTION_CHARS)
    {
        return Err(TierDefinitionsError::DescriptionTooLong {
            name,
            max: MAX_TIER_DESCRIPTION_CHARS,
        });
    }
    if description.is_none() && ComplexityTier::built_in(&name.to_uppercase()).is_none() {
        return Err(TierDefinitionsError::MissingDescription { name });
    }
    let has_newline = name.contains('\n')
        || name.contains('\r')
        || description
            .as_deref()
            .is_some_and(|text| text.contains('\n') || text.contains('\r'));
    if has_newline {
        return Err(TierDefinitionsError::Newline { name });
    }
    Ok(TierDefinition { name, description })
}

pub(super) fn validate_non_reasoning(
    non_reasoning: bool,
    custom: Option<&[TierDefinition]>,
    classifier: ClassifierKind,
    pools: &BTreeMap<String, TierPool>,
) -> Result<(), TierDefinitionsError> {
    let tier = ComplexityTier::NonReasoning.name();
    if !non_reasoning {
        if custom.is_none() && pools.contains_key(tier) {
            return Err(TierDefinitionsError::NonReasoningWithoutFlag { tier });
        }
        return Ok(());
    }
    if custom.is_some() {
        return Err(TierDefinitionsError::NonReasoningWithTiers);
    }
    if !matches!(
        classifier,
        ClassifierKind::Llm | ClassifierKind::Custom | ClassifierKind::Jev
    ) {
        return Err(TierDefinitionsError::NonReasoningClassifier { classifier });
    }
    if pools.get(tier).is_none_or(TierPool::is_empty) {
        return Err(TierDefinitionsError::NonReasoningWithoutModel { tier });
    }
    Ok(())
}

pub(super) fn validate_definitions(
    ladder: Option<&[TierDefinition]>,
    pools: &BTreeMap<String, TierPool>,
    labels: &BTreeMap<ComplexityTier, String>,
    checks: &TierChecks<'_>,
) -> Result<(), TierDefinitionsError> {
    let Some(defined) = ladder else {
        if checks.fallback_tier.is_some() {
            return Err(TierDefinitionsError::FallbackWithoutTiers);
        }
        return validate_opening(checks);
    };
    let names: Vec<&str> = defined
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    validate_definition_names(&names)?;
    if checks.classifier_type.rejects_custom_tiers() {
        return Err(TierDefinitionsError::ClassifierCannotEmitTiers {
            classifier: checks.classifier_type,
        });
    }
    let conflicts = definition_conflicts(checks, labels);
    if !conflicts.is_empty() {
        return Err(TierDefinitionsError::Conflicts { conflicts });
    }
    validate_defined_pools(&names, pools)?;
    validate_fallback(checks.fallback_tier, &names)
}

fn validate_definition_names(names: &[&str]) -> Result<(), TierDefinitionsError> {
    if !(MIN_TIER_DEFINITIONS..=MAX_TIER_DEFINITIONS).contains(&names.len()) {
        return Err(TierDefinitionsError::TierCount { count: names.len() });
    }
    let duplicated = duplicate_names(names);
    if !duplicated.is_empty() {
        return Err(TierDefinitionsError::DuplicateNames { names: duplicated });
    }
    Ok(())
}

fn duplicate_names(names: &[&str]) -> Vec<String> {
    let folded: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let mut duplicated: Vec<String> = names
        .iter()
        .zip(&folded)
        .filter(|(_, name)| folded.iter().filter(|other| *other == *name).count() > 1)
        .map(|(name, _)| (*name).to_owned())
        .collect();
    duplicated.sort();
    duplicated.dedup();
    duplicated
}

fn validate_defined_pools(
    names: &[&str],
    pools: &BTreeMap<String, TierPool>,
) -> Result<(), TierDefinitionsError> {
    let missing: Vec<String> = names
        .iter()
        .filter(|name| !pools.contains_key(**name))
        .map(|name| (*name).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(TierDefinitionsError::MissingTiers { names: missing });
    }
    let unknown: Vec<String> = pools
        .keys()
        .filter(|name| !names.contains(&name.as_str()))
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(TierDefinitionsError::UnknownTiers { names: unknown });
    }
    let mut empty: Vec<String> = names
        .iter()
        .filter(|name| pools.get(**name).is_none_or(TierPool::is_empty))
        .map(|name| (*name).to_owned())
        .collect();
    empty.sort();
    if empty.is_empty() {
        return Ok(());
    }
    Err(TierDefinitionsError::EmptyPools { names: empty })
}

fn validate_fallback(fallback: Option<&str>, names: &[&str]) -> Result<(), TierDefinitionsError> {
    let Some(configured) = fallback else {
        return Err(TierDefinitionsError::FallbackRequired);
    };
    if names.contains(&configured) {
        return Ok(());
    }
    Err(TierDefinitionsError::FallbackUnknown {
        tier: configured.to_owned(),
        defined: names.iter().map(|name| (*name).to_owned()).collect(),
    })
}

// these settings are written against the built-in ladder, which a custom set replaces
fn definition_conflicts(
    checks: &TierChecks<'_>,
    labels: &BTreeMap<ComplexityTier, String>,
) -> Vec<DefinitionConflict> {
    let mut conflicts = Vec::new();
    let features = checks.order_feature_labels();
    if !features.is_empty() {
        conflicts.push(DefinitionConflict::OrderFeatures(features));
    }
    if checks.llm_system_prompt.is_some() {
        conflicts.push(DefinitionConflict::SystemPrompt);
    }
    if checks.classification_rubric.is_some() {
        conflicts.push(DefinitionConflict::ClassificationRubric);
    }
    if checks.classifier_fallback == ClassifierFallback::DefaultModel {
        conflicts.push(DefinitionConflict::DefaultModelFallback);
    }
    if !labels.is_empty() {
        conflicts.push(DefinitionConflict::TierLabels);
    }
    conflicts
}

// a section-shaped rubric needs an LLM classifier and the prompt slot the system prompt would fill
const fn validate_opening(checks: &TierChecks<'_>) -> Result<(), TierDefinitionsError> {
    let field = if checks.classification_prompt.is_some() {
        "classification_prompt"
    } else if checks.classification_examples.is_some() {
        "classification_examples"
    } else {
        return Ok(());
    };
    if checks.llm_system_prompt.is_some() {
        return Err(TierDefinitionsError::OpeningWithSystemPrompt { field });
    }
    if !checks.classifier_type.uses_llm() {
        return Err(TierDefinitionsError::OpeningRequiresClassifier {
            field,
            classifier: checks.classifier_type,
        });
    }
    Ok(())
}
