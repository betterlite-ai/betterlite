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
    TierSet {
        definitions: definitions(input),
        pools: pools(input),
        labels: BTreeMap::new(),
        non_reasoning: input.non_reasoning.unwrap_or(false),
        fallback: None,
    }
}

fn definitions(input: &Input) -> Option<Vec<TierDefinition>> {
    let names = input.tier_definitions.as_ref()?;
    let mut found = Vec::with_capacity(names.len());
    for name in names {
        found.push(TierDefinition {
            name: name.clone(),
            description: None,
        });
    }
    Some(found)
}

fn pools(input: &Input) -> BTreeMap<String, TierPool> {
    input
        .tiers
        .iter()
        .map(|(name, models)| (name.clone(), pool(models)))
        .collect()
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
    let keyword_tier_rules = rules(input);
    let keyword_matching = if input.semantic_keyword_matching.unwrap_or(false) {
        KeywordMatching::Semantic
    } else {
        KeywordMatching::Lexical
    };
    let match_threshold = number(input.match_threshold.as_ref(), DEFAULT_THRESHOLD)?;
    let stall_window = count(input.stall_escalation_window.as_ref(), 6)?;
    let stall_repeat = count(input.stall_repeat_threshold.as_ref(), 3)?;
    let housekeeping_cheapest = input.housekeeping_cheapest.unwrap_or(true);
    let escalation_keywords = input.escalation_keywords.clone();
    let plan_mode_min_tier = input.plan_mode_min_tier.clone();
    let plan_mode_patterns = input.plan_mode_patterns.clone();
    let housekeeping_patterns = input.housekeeping_patterns.clone();
    let configured_markers = reminder_markers(input);
    let classifier_type = classifier(input);
    Ok(Settings {
        keyword_tier_rules,
        keyword_matching,
        embedding_model: input.embedding_model.clone(),
        match_threshold,
        stall_escalation_enabled: input.stall_escalation_enabled.unwrap_or(false),
        stall_escalation_window: stall_window,
        stall_repeat_threshold: stall_repeat,
        housekeeping_cheapest,
        escalation_keywords,
        plan_mode_min_tier,
        plan_mode_patterns,
        housekeeping_patterns,
        reminder_markers: configured_markers,
        classifier_type,
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
