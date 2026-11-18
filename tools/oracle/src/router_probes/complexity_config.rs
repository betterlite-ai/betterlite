use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as section;
use betterlite_config::sections::custom_dimensions as dimensions_section;
use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_router::complexity_config::checks::{Checks, Declared, Feature, StallWindow};
use betterlite_router::complexity_config::{Resolved, resolve};
use betterlite_router::tier_definitions::checks::{ClassifierFallback, ClassifierKind};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    #[serde(default)]
    settings: section::Config,
    #[serde(default)]
    tiers: TiersInput,
    #[serde(default)]
    knobs: Knobs,
}

// a payload arrives as JSON, whose numbers are finite, so the derived PartialEq is reflexive
impl Eq for Payload {}

// the tier map shape this probe reads; the tier model config keys belong to the tier probe
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TiersInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tier_models: Option<BTreeMap<String, tier_section::TierPool>>,
}

// the classifier kinds LiteLLM configures from this probe's keys; the other kinds reach the checks through the unit tests
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProbeClassifier {
    #[default]
    Heuristic,
    Llm,
    HeuristicFirst,
    Hybrid,
}

impl ProbeClassifier {
    const fn production(self) -> ClassifierKind {
        match self {
            Self::Heuristic => ClassifierKind::Heuristic,
            Self::Llm => ClassifierKind::Llm,
            Self::HeuristicFirst => ClassifierKind::HeuristicFirst,
            Self::Hybrid => ClassifierKind::Hybrid,
        }
    }
}

// the rest of the router configuration these validations read, supplied by the keys another task owns
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Knobs {
    #[serde(default)]
    classifier_type: ProbeClassifier,
    #[serde(default)]
    classifier_fallback: ClassifierFallback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fallback_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    classification_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    classification_examples: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    llm_system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    classification_rubric: Option<String>,
    #[serde(default)]
    keyword_rule_tiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    embedding_model: Option<String>,
    #[serde(default)]
    stall_escalation: StallWindow,
    // the production labels; no case sends plugins, since the Python side cannot build a plugin object
    #[serde(default)]
    features: Vec<Feature>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

// a payload arrives as JSON, whose numbers are finite, so the derived PartialEq is reflexive
impl Eq for Output {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Fields {
    tier_names: Vec<String>,
    escalation_keywords: Option<Vec<String>>,
    plan_mode_min_tier: Option<String>,
    plan_mode_patterns: Option<Vec<String>>,
    housekeeping_patterns: Option<Vec<String>>,
    heuristic_first_max_tier: Option<String>,
    hybrid_boundary_margin: Option<f64>,
    classification_prompt: Option<String>,
    classification_examples: Option<String>,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let knobs = &payload.knobs;
    let tiers = tier_section::Config {
        tier_definitions: None,
        non_reasoning_tier: false,
        tier_labels: BTreeMap::new(),
        tier_models: payload.tiers.tier_models.clone(),
    };
    let dimensions: [dimensions_section::Config; 0] = [];
    let weights = BTreeMap::new();
    let resolved = resolve(&Declared {
        settings: payload.settings.clone(),
        tiers,
        dimensions: &dimensions,
        dimension_weights: &weights,
        checks: router_checks(knobs),
    })
    .map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })?;
    Ok(ProbeOutput::ComplexityConfig(Output(Box::new(fields(
        &resolved,
    )))))
}

fn router_checks(knobs: &Knobs) -> Checks<'_> {
    Checks {
        classifier_type: knobs.classifier_type.production(),
        classifier_fallback: knobs.classifier_fallback,
        fallback_tier: knobs.fallback_tier.as_deref(),
        classification_prompt: knobs.classification_prompt.as_deref(),
        classification_examples: knobs.classification_examples.as_deref(),
        llm_system_prompt: knobs.llm_system_prompt.as_deref(),
        classification_rubric: knobs.classification_rubric.as_deref(),
        keyword_rule_tiers: &knobs.keyword_rule_tiers,
        embedding_model: knobs.embedding_model.as_deref(),
        stall_escalation: knobs.stall_escalation,
        features: knobs.features.as_slice(),
    }
}

fn fields(resolved: &Resolved) -> Fields {
    Fields {
        tier_names: resolved
            .tiers
            .tier_names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        escalation_keywords: resolved.escalation_keywords.clone(),
        plan_mode_min_tier: resolved.plan_mode_min_tier.clone(),
        plan_mode_patterns: resolved.plan_mode_patterns.clone(),
        housekeeping_patterns: resolved.housekeeping_patterns.clone(),
        heuristic_first_max_tier: resolved.heuristic_first_max_tier.clone(),
        hybrid_boundary_margin: resolved.hybrid_boundary_margin,
        classification_prompt: resolved.classification_prompt.clone(),
        classification_examples: resolved.classification_examples.clone(),
    }
}
