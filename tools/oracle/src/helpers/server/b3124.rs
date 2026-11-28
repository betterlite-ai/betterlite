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
        let features = router_features(&input.config);
        let checks = TierChecks {
            classifier_type: input.config.classifier_type,
            classifier_fallback: ClassifierFallback::Heuristic,
            fallback_tier: input.config.fallback_tier.as_deref(),
            classification_prompt: None,
            classification_examples: None,
            llm_system_prompt: None,
            classification_rubric: None,
            order_features: &features.order,
        };
        let tiers = TierSet::resolve(&config, &checks).or_config_error()?;
        let routing = CacheRoutingConfig {
            tiers: &tiers,
            features: &features.active,
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

struct RouterFeatures {
    // the settings whose presence a custom tier set refuses
    order: Vec<OrderFeature>,
    // the active router features, the same ones the production config layer hands CacheRoutingConfig
    active: Vec<Feature>,
}

fn router_features(config: &Config) -> RouterFeatures {
    let mut order = Vec::new();
    let mut active = Vec::new();
    if config.adaptive {
        order.push(OrderFeature::Adaptive);
        active.push(Feature::Adaptive);
    }
    if config
        .plugins
        .as_ref()
        .is_some_and(|plugins| !plugins.is_empty())
    {
        order.push(OrderFeature::Plugins);
        active.push(Feature::Plugins);
    }
    if config.session_affinity {
        order.push(OrderFeature::SessionAffinity);
        active.push(Feature::SessionAffinity);
    }
    if config
        .classification_mode
        .as_deref()
        .is_some_and(|mode| mode != "every_request")
    {
        active.push(Feature::UserTurnClassification);
    }
    RouterFeatures { order, active }
}
