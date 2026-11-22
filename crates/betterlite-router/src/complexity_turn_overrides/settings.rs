use betterlite_config::sections::classifier_message_text::MarkerPair;
use betterlite_config::sections::complexity_turn_overrides as section;

use crate::complexity_config::Resolved;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::model::TierSet;

use super::error::TurnOverrideError;

// the settings this task reads, after B329's cross-field checks and this section's own normalization
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub keyword_tier_rules: Vec<section::KeywordTierRule>,
    pub keyword_matching: section::KeywordMatching,
    pub embedding_model: Option<String>,
    pub match_threshold: f64,
    pub stall_escalation_enabled: bool,
    pub stall_escalation_window: u32,
    pub stall_repeat_threshold: u32,
    pub housekeeping_cheapest: bool,
    // B329 owns these four keys, so their normalized values arrive through its Resolved view
    pub escalation_keywords: Option<Vec<String>>,
    pub plan_mode_min_tier: Option<String>,
    pub plan_mode_patterns: Vec<String>,
    pub housekeeping_patterns: Vec<String>,
    // B384 owns reminder_markers and B329 owns classifier_type; the caller passes both in
    pub reminder_markers: Option<Vec<MarkerPair>>,
    pub classifier_type: ClassifierKind,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            keyword_tier_rules: Vec::new(),
            keyword_matching: section::KeywordMatching::default(),
            embedding_model: None,
            match_threshold: section::DEFAULT_MATCH_THRESHOLD,
            stall_escalation_enabled: section::DEFAULT_STALL_ENABLED,
            stall_escalation_window: section::DEFAULT_STALL_WINDOW,
            stall_repeat_threshold: section::DEFAULT_STALL_REPEAT,
            housekeeping_cheapest: section::DEFAULT_HOUSEKEEPING,
            escalation_keywords: None,
            plan_mode_min_tier: None,
            plan_mode_patterns: Vec::new(),
            housekeeping_patterns: Vec::new(),
            reminder_markers: None,
            classifier_type: ClassifierKind::Heuristic,
        }
    }
}

impl Settings {
    #[must_use]
    pub fn from_config(
        overrides: &section::Config,
        resolved: &Resolved,
        classifier_type: ClassifierKind,
        reminder_markers: Option<Vec<MarkerPair>>,
    ) -> Self {
        Self {
            keyword_tier_rules: overrides.keyword_tier_rules.clone().unwrap_or_default(),
            keyword_matching: overrides.keyword_matching,
            embedding_model: overrides.embedding_model.clone(),
            match_threshold: overrides.match_threshold,
            stall_escalation_enabled: overrides.stall_escalation_enabled,
            stall_escalation_window: overrides.stall_escalation_window,
            stall_repeat_threshold: overrides.stall_repeat_threshold,
            housekeeping_cheapest: overrides.housekeeping_cheapest,
            escalation_keywords: resolved.escalation_keywords.clone(),
            plan_mode_min_tier: resolved.plan_mode_min_tier.clone(),
            plan_mode_patterns: resolved.plan_mode_patterns.clone().unwrap_or_default(),
            housekeeping_patterns: resolved.housekeeping_patterns.clone().unwrap_or_default(),
            reminder_markers,
            classifier_type,
        }
    }
}

// _active_tier_severity: the position of a tier in the active severity order
pub fn tier_severity(catalog: &TierSet, tier: &Tier) -> Result<usize, TurnOverrideError> {
    let position = catalog
        .tier_names()
        .iter()
        .position(|name| *name == tier.name());
    match position {
        Some(rank) => Ok(rank),
        None => Err(TurnOverrideError::UnknownTier {
            tier: tier.name().to_owned(),
        }),
    }
}

// a configured tier name as the pipeline tier: a defined name for a custom set, else the built-in member
pub fn active_tier(catalog: &TierSet, name: &str) -> Result<Tier, TurnOverrideError> {
    if catalog.definitions.is_some() {
        return catalog
            .tier_names()
            .into_iter()
            .find(|defined| *defined == name)
            .map(|defined| Tier::Named(defined.to_owned()))
            .ok_or_else(|| unknown_tier(name));
    }
    ComplexityTier::built_in(name)
        .map(Tier::BuiltIn)
        .ok_or_else(|| unknown_tier(name))
}

fn unknown_tier(name: &str) -> TurnOverrideError {
    TurnOverrideError::UnknownTier {
        tier: name.to_owned(),
    }
}
