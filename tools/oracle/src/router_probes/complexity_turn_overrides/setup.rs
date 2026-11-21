use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::classifier_message_text::MarkerPair;
use betterlite_config::sections::complexity_turn_overrides::{KeywordMatching, KeywordTierRule};
use betterlite_router::complexity_turn_overrides::semantic::SemanticKeywordIndex;
use betterlite_router::complexity_turn_overrides::settings::Settings;
use betterlite_router::tier_definitions::checks::ClassifierKind;
use betterlite_router::tier_definitions::model::{
    TierDefinition, TierModelEntry, TierPool, TierSet,
};
use indexmap::IndexMap;

use super::Input;
use super::table::TableEmbedder;
use super::wire::{count, number};
use crate::router_probes::error::OperationError;

const DEFAULT_THRESHOLD: f64 = 0.5_f64;

pub(super) fn build_catalog(input: &Input) -> TierSet {
    let pools = input
        .tiers
        .iter()
        .map(|(name, models)| (name.clone(), pool(models)))
        .collect();
    let definitions = input.tier_definitions.as_ref().map(|names| {
        names
            .iter()
            .map(|name| TierDefinition {
                name: name.clone(),
                description: None,
            })
            .collect()
    });
    TierSet {
        definitions,
        pools,
        labels: BTreeMap::new(),
        non_reasoning: input.non_reasoning.unwrap_or(false),
        fallback: None,
    }
}

fn pool(models: &[String]) -> TierPool {
    TierPool::Pool(
        models
            .iter()
            .map(|model| TierModelEntry {
                model_name: model.clone(),
                params: IndexMap::new(),
            })
            .collect(),
    )
}

pub(super) fn build_settings(input: &Input) -> Result<Settings, OperationError> {
    Ok(Settings {
        keyword_tier_rules: rules(input),
        keyword_matching: if input.semantic_keyword_matching.unwrap_or(false) {
            KeywordMatching::Semantic
        } else {
            KeywordMatching::Lexical
        },
        embedding_model: input.embedding_model.clone(),
        match_threshold: number(input.match_threshold.as_ref(), DEFAULT_THRESHOLD)?,
        stall_escalation_enabled: input.stall_escalation_enabled.unwrap_or(false),
        stall_escalation_window: count(input.stall_escalation_window.as_ref(), 6)?,
        stall_escalation_repeat_threshold: count(
            input.stall_escalation_repeat_threshold.as_ref(),
            3,
        )?,
        route_housekeeping_to_cheapest_tier: input
            .route_housekeeping_to_cheapest_tier
            .unwrap_or(true),
        escalation_keywords: input.escalation_keywords.clone(),
        plan_mode_min_tier: input.plan_mode_min_tier.clone(),
        plan_mode_patterns: input.plan_mode_patterns.clone(),
        housekeeping_patterns: input.housekeeping_patterns.clone(),
        reminder_markers: reminder_markers(input),
        classifier_type: classifier(input),
    })
}

fn rules(input: &Input) -> Vec<KeywordTierRule> {
    input
        .keyword_tier_rules
        .iter()
        .map(|rule| KeywordTierRule {
            keywords: rule.keywords.clone(),
            tier: rule.tier.clone(),
        })
        .collect()
}

fn reminder_markers(input: &Input) -> Option<Vec<MarkerPair>> {
    input.reminder_markers.as_ref().map(|pairs| {
        pairs
            .iter()
            .map(|pair| MarkerPair {
                open: pair.open.clone(),
                close: pair.close.clone(),
            })
            .collect()
    })
}

fn classifier(input: &Input) -> ClassifierKind {
    if input.classifier_type.as_deref() == Some("custom") {
        ClassifierKind::Custom
    } else {
        ClassifierKind::Heuristic
    }
}

pub(super) fn semantic_index(
    input: &Input,
    settings: &Settings,
    catalog: &TierSet,
    embedder: &Arc<TableEmbedder>,
) -> Option<SemanticKeywordIndex<TableEmbedder>> {
    if !input.semantic_keyword_matching.unwrap_or(false) || settings.keyword_tier_rules.is_empty() {
        return None;
    }
    SemanticKeywordIndex::new(settings, catalog, Arc::clone(embedder)).ok()
}
