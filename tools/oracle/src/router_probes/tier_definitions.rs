use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as section;
use betterlite_router::complexity_scoring::tier::tier_severity_order;
use betterlite_router::tier_definitions::checks::{
    ClassifierFallback, ClassifierKind, OrderFeature, TierChecks,
};
use betterlite_router::tier_definitions::model::{TierPool, TierSet};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    // the section's typed view, so the probe reads tiers exactly as a config file does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config: Option<section::Config>,
    #[serde(default)]
    knobs: Knobs,
    #[serde(default)]
    queries: Vec<String>,
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
    order_features: Vec<ProbeFeature>,
}

// the classifier kinds LiteLLM configures from this probe's keys; the other kinds reach the checks through the unit tests
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProbeClassifier {
    #[default]
    Heuristic,
    Llm,
}

impl ProbeClassifier {
    const fn production(self) -> ClassifierKind {
        match self {
            Self::Heuristic => ClassifierKind::Heuristic,
            Self::Llm => ClassifierKind::Llm,
        }
    }
}

// plugins stay out: a case would have to build a Python plugin object for the reference side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProbeFeature {
    Adaptive,
    SessionAffinity,
    EscalationKeywords,
    StallEscalation,
}

impl ProbeFeature {
    const fn production(self) -> OrderFeature {
        match self {
            Self::Adaptive => OrderFeature::Adaptive,
            Self::SessionAffinity => OrderFeature::SessionAffinity,
            Self::EscalationKeywords => OrderFeature::EscalationKeywords,
            Self::StallEscalation => OrderFeature::StallEscalation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

// A bool on the wire; the newtype keeps struct_excessive_bools off the output view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct Flag(bool);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Fields {
    custom: Flag,
    non_reasoning: Flag,
    uses_llm_classifier: Flag,
    tier_names: Vec<String>,
    wire_labels: Vec<String>,
    severity_order: Vec<String>,
    labeled_tiers: Vec<LabeledTier>,
    fallback_tier: Option<String>,
    pools: BTreeMap<String, PoolView>,
    resolved_tiers: Vec<Option<String>>,
    label_tiers: Vec<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LabeledTier {
    tier: String,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PoolView {
    single: bool,
    entries: Vec<EntryView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EntryView {
    model_name: String,
    params: section::TierParams,
}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let config = payload.config.clone().unwrap_or_default();
    let features: Vec<OrderFeature> = payload
        .knobs
        .order_features
        .iter()
        .map(|feature| feature.production())
        .collect();
    let checks = TierChecks {
        classifier_type: payload.knobs.classifier_type.production(),
        classifier_fallback: payload.knobs.classifier_fallback,
        fallback_tier: payload.knobs.fallback_tier.as_deref(),
        classification_prompt: payload.knobs.classification_prompt.as_deref(),
        classification_examples: payload.knobs.classification_examples.as_deref(),
        llm_system_prompt: payload.knobs.llm_system_prompt.as_deref(),
        classification_rubric: payload.knobs.classification_rubric.as_deref(),
        order_features: &features,
    };
    let set =
        TierSet::resolve(&config, &checks).map_err(|source| OperationError::InvalidRequest {
            message: source.to_string(),
        })?;
    let fields = fields(&set, payload);
    Ok(ProbeOutput::TierDefinitions(Output(Box::new(fields))))
}

fn fields(set: &TierSet, payload: &Payload) -> Fields {
    Fields {
        custom: Flag(set.definitions.is_some()),
        non_reasoning: Flag(set.non_reasoning),
        uses_llm_classifier: Flag(payload.knobs.classifier_type.production().uses_llm()),
        tier_names: set.tier_names().into_iter().map(str::to_owned).collect(),
        wire_labels: set.wire_labels().into_iter().map(str::to_owned).collect(),
        severity_order: tier_severity_order(set.non_reasoning)
            .iter()
            .map(|tier| tier.name().to_owned())
            .collect(),
        labeled_tiers: set
            .labeled_tiers()
            .into_iter()
            .map(|(tier, label)| LabeledTier {
                tier: tier.name().to_owned(),
                label: label.to_owned(),
            })
            .collect(),
        fallback_tier: set.fallback.clone(),
        pools: set
            .pools
            .iter()
            .map(|(tier, pool)| (tier.clone(), pool_view(pool)))
            .collect(),
        resolved_tiers: payload
            .queries
            .iter()
            .map(|query| {
                set.resolve_classified_tier(query)
                    .map(|tier| tier.name().to_owned())
            })
            .collect(),
        label_tiers: payload
            .queries
            .iter()
            .map(|query| set.tier_for_label(query).map(|tier| tier.name().to_owned()))
            .collect(),
    }
}

fn pool_view(pool: &TierPool) -> PoolView {
    PoolView {
        single: matches!(pool, TierPool::Single(_)),
        entries: pool
            .models()
            .iter()
            .map(|entry| EntryView {
                model_name: entry.model_name.clone(),
                params: entry.params.clone(),
            })
            .collect(),
    }
}
