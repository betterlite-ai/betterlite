use std::collections::BTreeSet;

use crate::complexity_config::checks::{Checks, Feature};
use crate::complexity_config::error::ComplexityConfigError;
use crate::complexity_config::normalize::Normalized;
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::model::{TierPool, TierSet};

pub fn cross_field(
    normalized: &Normalized,
    checks: &Checks<'_>,
    set: &TierSet,
) -> Result<(), ComplexityConfigError> {
    plan_mode_floor(normalized, set)?;
    heuristic_first_ceiling(normalized, checks, set)?;
    hybrid_boundary_margin(normalized, checks)?;
    keyword_rule_tiers(checks, set)?;
    adaptive_pools(checks, set)?;
    semantic_matching(checks)?;
    plugins_with_adaptive(checks)?;
    stall_escalation(checks)?;
    Ok(())
}

fn plan_mode_floor(normalized: &Normalized, set: &TierSet) -> Result<(), ComplexityConfigError> {
    let Some(tier) = &normalized.plan_mode_min_tier else {
        return Ok(());
    };
    let names = set.tier_names();
    if !names.contains(&tier.as_str()) {
        return Err(ComplexityConfigError::PlanModeUnknownTier {
            tier: tier.clone(),
            names: names.join(", "),
        });
    }
    if !set.pools.contains_key(tier.as_str()) {
        return Err(ComplexityConfigError::PlanModeWithoutModel { tier: tier.clone() });
    }
    Ok(())
}

fn heuristic_first_ceiling(
    normalized: &Normalized,
    checks: &Checks<'_>,
    set: &TierSet,
) -> Result<(), ComplexityConfigError> {
    let threshold = normalized.heuristic_first_max_tier.as_deref();
    if checks.classifier_type != ClassifierKind::HeuristicFirst {
        if threshold.is_some() {
            return Err(ComplexityConfigError::HeuristicFirstUnused {
                classifier: checks.classifier_type,
            });
        }
        return Ok(());
    }
    let Some(tier) = threshold else {
        return Err(ComplexityConfigError::HeuristicFirstRequired);
    };
    let names = set.tier_names();
    if !names.contains(&tier) {
        return Err(ComplexityConfigError::HeuristicFirstUnknownTier {
            tier: tier.to_owned(),
            names: names.join(", "),
        });
    }
    if names.last().copied() == Some(tier) {
        return Err(ComplexityConfigError::HeuristicFirstHighest {
            tier: tier.to_owned(),
        });
    }
    if !set.pools.contains_key(tier) {
        return Err(ComplexityConfigError::HeuristicFirstWithoutModel {
            tier: tier.to_owned(),
        });
    }
    Ok(())
}

fn hybrid_boundary_margin(
    normalized: &Normalized,
    checks: &Checks<'_>,
) -> Result<(), ComplexityConfigError> {
    if checks.classifier_type != ClassifierKind::Hybrid {
        if normalized.hybrid_boundary_margin.is_some() {
            return Err(ComplexityConfigError::HybridMarginUnused {
                classifier: checks.classifier_type,
            });
        }
        return Ok(());
    }
    if normalized.hybrid_boundary_margin.is_none() {
        return Err(ComplexityConfigError::HybridMarginRequired);
    }
    Ok(())
}

fn keyword_rule_tiers(checks: &Checks<'_>, set: &TierSet) -> Result<(), ComplexityConfigError> {
    if checks.keyword_rule_tiers.is_empty() {
        return Ok(());
    }
    let names = set.tier_names();
    let valid: BTreeSet<&str> = names.iter().copied().collect();
    let unknown: BTreeSet<&str> = checks
        .keyword_rule_tiers
        .iter()
        .map(String::as_str)
        .filter(|tier| !valid.contains(tier))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(ComplexityConfigError::KeywordRuleUnknownTiers {
        tiers: unknown.into_iter().map(str::to_owned).collect(),
        valid: names.join(", "),
    })
}

// LiteLLM's adaptive check sees a tier's entries as a list, so only an explicitly empty list is empty here
fn adaptive_pools(checks: &Checks<'_>, set: &TierSet) -> Result<(), ComplexityConfigError> {
    if !checks.features.contains(&Feature::Adaptive) {
        return Ok(());
    }
    let is_empty = |pool: &TierPool| matches!(pool, TierPool::Pool(entries) if entries.is_empty());
    if set.pools.values().all(is_empty) {
        return Err(ComplexityConfigError::AdaptiveWithoutPools);
    }
    let empty: Vec<String> = set
        .pools
        .iter()
        .filter(|&(_, pool)| is_empty(pool))
        .map(|(tier, _)| tier.clone())
        .collect();
    if empty.is_empty() {
        return Ok(());
    }
    Err(ComplexityConfigError::AdaptiveEmptyPools { tiers: empty })
}

fn semantic_matching(checks: &Checks<'_>) -> Result<(), ComplexityConfigError> {
    if !checks.features.contains(&Feature::SemanticKeywordMatching) {
        return Ok(());
    }
    if checks.embedding_model.is_none() {
        return Err(ComplexityConfigError::SemanticWithoutEmbeddingModel);
    }
    if checks.keyword_rule_tiers.is_empty() {
        return Err(ComplexityConfigError::SemanticWithoutKeywordRules);
    }
    Ok(())
}

fn plugins_with_adaptive(checks: &Checks<'_>) -> Result<(), ComplexityConfigError> {
    if checks.features.contains(&Feature::Plugins) && checks.features.contains(&Feature::Adaptive) {
        return Err(ComplexityConfigError::PluginsWithAdaptive);
    }
    Ok(())
}

fn stall_escalation(checks: &Checks<'_>) -> Result<(), ComplexityConfigError> {
    if !checks.features.contains(&Feature::StallEscalation) {
        return Ok(());
    }
    if checks.features.contains(&Feature::SessionAffinity)
        || checks.features.contains(&Feature::UserTurnClassification)
    {
        return Err(ComplexityConfigError::StallWithHeldDecision);
    }
    let stall = checks.stall_escalation;
    if stall.repeat_threshold > stall.window {
        return Err(ComplexityConfigError::StallThresholdAboveWindow {
            threshold: stall.repeat_threshold,
            window: stall.window,
        });
    }
    Ok(())
}
