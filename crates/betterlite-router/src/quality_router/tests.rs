#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::quality_router as section;
use betterlite_models::decisions::Json;
use betterlite_prompts::message_content::Message;

use crate::quality_router::QualityRouter;
use crate::quality_router::decision::Decision;

mod quality_router_behaviors;
mod quality_router_boundaries;

const CONFIG_VALID: &str =
    include_str!("../../../betterlite-config/fuzz/corpus/config_yaml/quality-router-valid.yaml");
const CONFIG_INVALID: &str =
    include_str!("../../../betterlite-config/fuzz/corpus/config_yaml/quality-router-invalid.yaml");

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// one deployment per available model, in model_list order
fn deployment(name: &str, tier: i64) -> section::Deployment {
    section::Deployment {
        model_name: name.to_owned(),
        config: None,
        preferences: Some(section::Preferences {
            quality_tier: tier,
            keywords: Vec::new(),
            order: None,
            extras: BTreeMap::new(),
        }),
        input_cost_per_token: None,
    }
}

fn keyword_decision(name: &str, model: &str, keyword: &str, quality_tier: i64) -> Decision {
    Decision::Keyword {
        router_model_name: name.to_owned(),
        routed_model: model.to_owned(),
        matched_keyword: keyword.to_owned(),
        quality_tier,
    }
}

fn fallback_decision(name: &str, model: &str) -> Decision {
    Decision::DefaultFallback {
        router_model_name: name.to_owned(),
        routed_model: model.to_owned(),
    }
}

fn keyword(mut deployment: section::Deployment, word: &str) -> section::Deployment {
    if let Some(preferences) = deployment.preferences.as_mut() {
        preferences.keywords.push(word.to_owned());
    }
    deployment
}

fn ordered(mut deployment: section::Deployment, order: i64) -> section::Deployment {
    if let Some(preferences) = deployment.preferences.as_mut() {
        preferences.order = Some(order);
    }
    deployment
}

fn priced(mut deployment: section::Deployment, cost: i64, scale: u32) -> section::Deployment {
    deployment.input_cost_per_token = Some(rust_decimal::Decimal::new(cost, scale));
    deployment
}

fn router_with(
    available: &[&str],
    mapping: Option<BTreeMap<String, i64>>,
    default_model: Option<&str>,
    deployments: &[section::Deployment],
) -> QualityRouter {
    let config = section::RouterConfig {
        available_models: strings(available),
        default_model: default_model.map(str::to_owned),
        complexity_to_quality: mapping.unwrap_or_else(section::default_complexity_to_quality),
        extras: BTreeMap::new(),
    };
    QualityRouter::new("qr", Some(&config), None, deployments).unwrap()
}

// the pinned complexity_to_quality table, so SIMPLE lands on quality tier 1
fn router(available: &[&str], deployments: &[section::Deployment]) -> QualityRouter {
    router_with(available, None, None, deployments)
}

fn table(pairs: &[(&str, i64)]) -> BTreeMap<String, i64> {
    pairs
        .iter()
        .map(|(tier, quality)| ((*tier).to_owned(), *quality))
        .collect()
}

fn messages(source: &str) -> Vec<Message> {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn text_settings(source: &str) -> Result<section::Settings, String> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.quality_router)
    .map_err(|error| error.to_string())
}
