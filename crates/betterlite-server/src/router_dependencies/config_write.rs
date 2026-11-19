use std::collections::BTreeMap;

use betterlite_config::sections::complexity_config as settings_section;
use betterlite_config::sections::custom_dimensions as dimensions_section;
use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::complexity_config::checks::{
    Checks, Declared, Feature, STALL_REPEAT_DEFAULT, STALL_WINDOW_DEFAULT, StallWindow,
};
use betterlite_router::complexity_config::error::ComplexityConfigError;
use betterlite_router::complexity_config::resolve;
use betterlite_router::tier_definitions::checks::{ClassifierFallback, ClassifierKind};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::error::ComplexityConfigWriteError;

// the key a write names in its own errors when the failing location has no field of its own
const CONFIG_ROOT: &str = "complexity_router_config";

// LiteLLM parses a written config with the router's own model, and betterlite's model is the loader's section config
pub fn validate_complexity_router_config_write(
    complexity_router_config: Option<&Json>,
) -> Result<(), ComplexityConfigWriteError> {
    let Some(Json::Object(fields)) = complexity_router_config else {
        return match complexity_router_config {
            None | Some(Json::Null) => Ok(()),
            Some(_) => Err(ComplexityConfigWriteError::NotAnObject),
        };
    };
    // a top-level null takes the field's default, the way the loader reads it
    let fields = without_nulls(fields);
    // the fields are destructured so the checks borrow them one by one instead of the whole record
    let Inputs {
        settings,
        tiers,
        dimensions,
        dimension_weights,
        raw,
        features,
        keyword_rule_tiers,
    } = decode_declared(&fields)?;
    let checks = decode_checks(&raw, &features, &keyword_rule_tiers);
    let declared = Declared {
        settings,
        tiers,
        dimensions: &dimensions,
        dimension_weights: &dimension_weights,
        checks,
    };
    resolve(&declared).map_err(|source| ComplexityConfigWriteError::Invalid {
        location: field_location(&source).to_owned(),
        source,
    })?;
    Ok(())
}

struct Inputs {
    settings: settings_section::Config,
    tiers: tier_section::Config,
    dimensions: Vec<dimensions_section::Config>,
    dimension_weights: BTreeMap<String, f64>,
    raw: RawChecks,
    features: Vec<Feature>,
    keyword_rule_tiers: Vec<String>,
}

fn decode_declared(fields: &JsonObject) -> Result<Inputs, ComplexityConfigWriteError> {
    let raw: RawChecks = decode_object(fields, CONFIG_ROOT)?;
    let keyword_rule_tiers = raw
        .keyword_tier_rules
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|rule| rule.tier.clone())
        .collect();
    let features = order_features(&raw);
    Ok(Inputs {
        settings: decode_object(fields, CONFIG_ROOT)?,
        tiers: decode_object(fields, CONFIG_ROOT)?,
        dimensions: decode_optional_field(fields, "custom_dimensions")?.unwrap_or_default(),
        dimension_weights: decode_optional_field(fields, "dimension_weights")?.unwrap_or_default(),
        raw,
        features,
        keyword_rule_tiers,
    })
}

fn decode_checks<'config>(
    raw: &'config RawChecks,
    features: &'config [Feature],
    keyword_rule_tiers: &'config [String],
) -> Checks<'config> {
    let llm = raw.classifier_llm_config.as_ref();
    Checks {
        classifier_type: raw.classifier_type.unwrap_or_default(),
        classifier_fallback: raw.classifier_fallback.unwrap_or_default(),
        fallback_tier: raw.fallback_tier.as_deref(),
        classification_prompt: raw.classification_prompt.as_deref(),
        classification_examples: raw.classification_examples.as_deref(),
        llm_system_prompt: llm.and_then(|config| config.system_prompt.as_deref()),
        classification_rubric: llm.and_then(|config| config.classification_rubric.as_deref()),
        keyword_rule_tiers,
        embedding_model: raw.embedding_model.as_deref(),
        stall_escalation: StallWindow {
            window: raw.stall_escalation_window.unwrap_or(STALL_WINDOW_DEFAULT),
            repeat_threshold: raw
                .stall_escalation_repeat_threshold
                .unwrap_or(STALL_REPEAT_DEFAULT),
        },
        features,
    }
}

// LiteLLM's cross-field rules read these settings by truthiness, so a false flag or an empty list is off
fn order_features(raw: &RawChecks) -> Vec<Feature> {
    let mut features = Vec::new();
    if raw.adaptive == Some(true) {
        features.push(Feature::Adaptive);
    }
    if raw
        .plugins
        .as_deref()
        .is_some_and(|plugins| !plugins.is_empty())
    {
        features.push(Feature::Plugins);
    }
    if raw.session_affinity == Some(true) {
        features.push(Feature::SessionAffinity);
    }
    if raw.classification_mode == Some(ClassificationMode::UserTurn) {
        features.push(Feature::UserTurnClassification);
    }
    if raw.stall_escalation_enabled == Some(true) {
        features.push(Feature::StallEscalation);
    }
    if raw.semantic_keyword_matching == Some(true) {
        features.push(Feature::SemanticKeywordMatching);
    }
    features
}

// LiteLLM reports a field constraint at the field and a model-level rule at the model root
fn field_location(error: &ComplexityConfigError) -> &'static str {
    match error {
        ComplexityConfigError::BlankText { field }
        | ComplexityConfigError::TextTooLong { field, .. } => field,
        ComplexityConfigError::HybridMarginOutOfRange { .. } => "hybrid_boundary_margin",
        ComplexityConfigError::Tiers(_)
        | ComplexityConfigError::Dimensions(_)
        | ComplexityConfigError::PlanModeUnknownTier { .. }
        | ComplexityConfigError::PlanModeWithoutModel { .. }
        | ComplexityConfigError::HeuristicFirstUnused { .. }
        | ComplexityConfigError::HeuristicFirstRequired
        | ComplexityConfigError::HeuristicFirstUnknownTier { .. }
        | ComplexityConfigError::HeuristicFirstHighest { .. }
        | ComplexityConfigError::HeuristicFirstWithoutModel { .. }
        | ComplexityConfigError::HybridMarginUnused { .. }
        | ComplexityConfigError::HybridMarginRequired
        | ComplexityConfigError::KeywordRuleUnknownTiers { .. }
        | ComplexityConfigError::AdaptiveWithoutPools
        | ComplexityConfigError::AdaptiveEmptyPools { .. }
        | ComplexityConfigError::SemanticWithoutEmbeddingModel
        | ComplexityConfigError::SemanticWithoutKeywordRules
        | ComplexityConfigError::PluginsWithAdaptive
        | ComplexityConfigError::StallWithHeldDecision
        | ComplexityConfigError::StallThresholdAboveWindow { .. }
        | ComplexityConfigError::MisplacedTierParams { .. } => CONFIG_ROOT,
    }
}

// a top-level null takes the field's default, so it is dropped before the typed sections decode it
fn without_nulls(fields: &JsonObject) -> JsonObject {
    let mut declared = JsonObject::new();
    for (key, value) in fields {
        if !matches!(value, Json::Null) {
            declared.insert(key.clone(), value.clone());
        }
    }
    declared
}

fn decode_object<Decoded>(
    fields: &JsonObject,
    location: &str,
) -> Result<Decoded, ComplexityConfigWriteError>
where
    Decoded: DeserializeOwned,
{
    let text =
        serde_json::to_string(fields).map_err(|source| ComplexityConfigWriteError::Unreadable {
            message: source.to_string(),
        })?;
    decode_text(&text, "", location)
}

fn decode_optional_field<Decoded>(
    fields: &JsonObject,
    key: &str,
) -> Result<Option<Decoded>, ComplexityConfigWriteError>
where
    Decoded: DeserializeOwned,
{
    let Some(value) = fields.get(key) else {
        return Ok(None);
    };
    let text =
        serde_json::to_string(value).map_err(|source| ComplexityConfigWriteError::Unreadable {
            message: source.to_string(),
        })?;
    let decoded = decode_text(&text, key, key)?;
    Ok(Some(decoded))
}

// the prefix names the field a nested value was read from, and the fallback names the value itself
fn decode_text<Decoded>(
    text: &str,
    prefix: &str,
    fallback: &str,
) -> Result<Decoded, ComplexityConfigWriteError>
where
    Decoded: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_str(text);
    serde_path_to_error::deserialize(&mut deserializer).map_err(|source| {
        let path = source.path().to_string();
        let location = if path.is_empty() {
            fallback.to_owned()
        } else if prefix.is_empty() {
            path
        } else {
            format!("{prefix}.{path}")
        };
        ComplexityConfigWriteError::InvalidField {
            location,
            message: source.into_inner().to_string(),
        }
    })
}

// the settings the cross-field rules read that the section configs do not carry
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawChecks {
    classifier_type: Option<ClassifierKind>,
    classifier_fallback: Option<ClassifierFallback>,
    fallback_tier: Option<String>,
    classification_prompt: Option<String>,
    classification_examples: Option<String>,
    classifier_llm_config: Option<RawLlmConfig>,
    keyword_tier_rules: Option<Vec<RawKeywordRule>>,
    embedding_model: Option<String>,
    stall_escalation_window: Option<u32>,
    stall_escalation_repeat_threshold: Option<u32>,
    adaptive: Option<bool>,
    session_affinity: Option<bool>,
    stall_escalation_enabled: Option<bool>,
    semantic_keyword_matching: Option<bool>,
    classification_mode: Option<ClassificationMode>,
    plugins: Option<Vec<Json>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawLlmConfig {
    system_prompt: Option<String>,
    classification_rubric: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawKeywordRule {
    tier: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClassificationMode {
    EveryRequest,
    UserTurn,
}
