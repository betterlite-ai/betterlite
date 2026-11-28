use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions as tier_section;
use betterlite_models::decisions::Json;
use betterlite_router::complexity_config::checks::Feature;
use betterlite_router::complexity_scoring::decision::RoutingDecision;
use betterlite_router::tier_definitions::checks::ClassifierFallback;
use betterlite_router::tier_definitions::checks::ClassifierKind;
use betterlite_router::tier_definitions::checks::OrderFeature;
use betterlite_router::tier_definitions::checks::TierChecks;
use betterlite_router::tier_definitions::model::TierSet;
use betterlite_server::routing::cache_alternatives::config::CacheRoutingConfig;
use betterlite_server::routing::cache_alternatives::eligibility::eligible_models;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::error::OrHelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

// the ComplexityRouterConfig keys eligible_models reads, as LiteLLM's model_validate takes them
#[derive(Deserialize)]
struct Config {
    #[serde(default, alias = "tiers")]
    tier_models: Option<BTreeMap<String, tier_section::TierPool>>,
    #[serde(default)]
    tier_definitions: Option<Vec<tier_section::TierDefinition>>,
    #[serde(default, alias = "enable_non_reasoning_tier")]
    non_reasoning_tier: bool,
    #[serde(default)]
    classifier_type: ClassifierKind,
    #[serde(default)]
    fallback_tier: Option<String>,
    #[serde(default)]
    adaptive: bool,
    #[serde(default)]
    session_affinity: bool,
    #[serde(default)]
    plugins: Option<Vec<Json>>,
    #[serde(default)]
    classification_mode: Option<String>,
}

#[derive(Deserialize)]
struct Input {
    config: Config,
    decision: RoutingDecision,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let config = tier_config(&input.config);
        let order_features = order_features(&input.config);
        let checks = TierChecks {
            classifier_type: input.config.classifier_type,
            classifier_fallback: ClassifierFallback::Heuristic,
            fallback_tier: input.config.fallback_tier.as_deref(),
            classification_prompt: None,
            classification_examples: None,
            llm_system_prompt: None,
            classification_rubric: None,
            order_features: &order_features,
        };
        let tiers = TierSet::resolve(&config, &checks).or_config_error()?;
        let features = features(&input.config);
        let routing = CacheRoutingConfig {
            tiers: &tiers,
            features: &features,
        };
        let names = eligible_models(&routing, &input.decision);
        let eligible: Vec<[&str; 2]> = names
            .iter()
            .map(|(tier, model)| [tier.as_str(), model.as_str()])
            .collect();
        Ok(json!({ "eligible": eligible }))
    }
}

fn tier_config(config: &Config) -> tier_section::Config {
    tier_section::Config {
        tier_definitions: config.tier_definitions.clone(),
        non_reasoning_tier: config.non_reasoning_tier,
        tier_labels: BTreeMap::new(),
        tier_models: config.tier_models.clone(),
    }
}

// the settings whose presence a custom tier set refuses
fn order_features(config: &Config) -> Vec<OrderFeature> {
    let mut order_features = Vec::new();
    if config.adaptive {
        order_features.push(OrderFeature::Adaptive);
    }
    if config
        .plugins
        .as_ref()
        .is_some_and(|plugins| !plugins.is_empty())
    {
        order_features.push(OrderFeature::Plugins);
    }
    if config.session_affinity {
        order_features.push(OrderFeature::SessionAffinity);
    }
    order_features
}

// the active router features, the same ones the production config layer hands CacheRoutingConfig
fn features(config: &Config) -> Vec<Feature> {
    let mut features = Vec::new();
    if config.adaptive {
        features.push(Feature::Adaptive);
    }
    if config
        .plugins
        .as_ref()
        .is_some_and(|plugins| !plugins.is_empty())
    {
        features.push(Feature::Plugins);
    }
    if config.session_affinity {
        features.push(Feature::SessionAffinity);
    }
    let user_turn = config
        .classification_mode
        .as_deref()
        .is_some_and(|mode| mode != "every_request");
    if user_turn {
        features.push(Feature::UserTurnClassification);
    }
    features
}
