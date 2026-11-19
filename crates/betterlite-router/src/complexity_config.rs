use crate::custom_dimensions::{self, Validated};
use crate::tier_definitions::checks::{OrderFeature, TierChecks};
use crate::tier_definitions::model::TierSet;

pub mod checks;
pub mod error;
pub mod keys;
pub mod normalize;
pub mod placement;
pub mod validate;

mod tests;

use checks::{Checks, Declared, Feature};
use error::ComplexityConfigError;
use normalize::Normalized;

// the tier set, the validated dimensions and the six settings after their normalizers, which consumers route with
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub tiers: TierSet,
    pub dimensions: Validated,
    pub escalation_keywords: Option<Vec<String>>,
    pub plan_mode_min_tier: Option<String>,
    pub plan_mode_patterns: Option<Vec<String>>,
    pub housekeeping_patterns: Option<Vec<String>>,
    pub heuristic_first_max_tier: Option<String>,
    pub hybrid_boundary_margin: Option<f64>,
    pub classification_prompt: Option<String>,
    pub classification_examples: Option<String>,
}

pub fn resolve(declared: &Declared<'_>) -> Result<Resolved, ComplexityConfigError> {
    let normalized = normalize::apply(&declared.settings, &declared.checks)?;
    let features = order_features(&declared.checks, &normalized);
    let tier_checks = TierChecks {
        classifier_type: declared.checks.classifier_type,
        classifier_fallback: declared.checks.classifier_fallback,
        fallback_tier: normalized.fallback_tier.as_deref(),
        classification_prompt: normalized.classification_prompt.as_deref(),
        classification_examples: normalized.classification_examples.as_deref(),
        llm_system_prompt: declared.checks.llm_system_prompt,
        classification_rubric: declared.checks.classification_rubric,
        order_features: &features,
    };
    let tiers = TierSet::resolve(&declared.tiers, &tier_checks)?;
    let dimensions = custom_dimensions::validate(
        declared.dimensions,
        declared.checks.classifier_type.name(),
        declared.dimension_weights,
    )?;
    validate::cross_field(&normalized, &declared.checks, &tiers)?;
    placement::validate(&tiers)?;
    Ok(Resolved {
        tiers,
        dimensions,
        escalation_keywords: normalized.escalation_keywords,
        plan_mode_min_tier: normalized.plan_mode_min_tier,
        plan_mode_patterns: normalized.plan_mode_patterns,
        housekeeping_patterns: normalized.housekeeping_patterns,
        heuristic_first_max_tier: normalized.heuristic_first_max_tier,
        hybrid_boundary_margin: normalized.hybrid_boundary_margin,
        classification_prompt: normalized.classification_prompt,
        classification_examples: normalized.classification_examples,
    })
}

// the built-in-ladder settings whose presence a custom tier set rejects, read the way LiteLLM's truthiness sees them
fn order_features(checks: &Checks<'_>, normalized: &Normalized) -> Vec<OrderFeature> {
    let mut features = Vec::new();
    for feature in checks.features {
        match feature {
            Feature::Adaptive => features.push(OrderFeature::Adaptive),
            Feature::Plugins => features.push(OrderFeature::Plugins),
            Feature::SessionAffinity => features.push(OrderFeature::SessionAffinity),
            Feature::StallEscalation => features.push(OrderFeature::StallEscalation),
            Feature::UserTurnClassification | Feature::SemanticKeywordMatching => {}
        }
    }
    if normalized
        .escalation_keywords
        .as_deref()
        .is_some_and(|keywords| !keywords.is_empty())
    {
        features.push(OrderFeature::EscalationKeywords);
    }
    features
}
