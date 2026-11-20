use std::collections::BTreeMap;

use betterlite_config::sections::tier_definitions::{TierEntry, TierPool as WireTierPool};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::tier_definitions::model::{TierModelEntry, TierPool, TierSet};
use indexmap::IndexMap;
use serde_json::Number;

use super::super::wire::DeploymentInput;

pub(super) fn tier_set(tiers: &IndexMap<String, WireTierPool>) -> TierSet {
    TierSet {
        definitions: None,
        pools: tiers
            .iter()
            .map(|(name, pool)| (name.clone(), tier_pool(pool)))
            .collect(),
        labels: BTreeMap::new(),
        non_reasoning: false,
        fallback: None,
    }
}

fn tier_pool(pool: &WireTierPool) -> TierPool {
    match pool {
        WireTierPool::Single(entry) => TierPool::Single(tier_entry(entry)),
        WireTierPool::Pool(entries) => TierPool::Pool(entries.iter().map(tier_entry).collect()),
    }
}

fn tier_entry(entry: &TierEntry) -> TierModelEntry {
    match entry {
        TierEntry::Model(name) => TierModelEntry {
            model_name: name.clone(),
            params: IndexMap::new(),
        },
        TierEntry::Detailed(detailed) => TierModelEntry {
            model_name: detailed.model_name.clone(),
            params: detailed.litellm_params.clone(),
        },
    }
}

pub(super) fn deployments(entries: &[DeploymentInput]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for entry in entries {
        catalog.push(deployment_spec(entry));
    }
    catalog
}

// the deployment's litellm_params and model_info as the YAML declares them
fn deployment_spec(entry: &DeploymentInput) -> DeploymentSpec {
    let mut params = JsonObject::new();
    params.insert(
        "model".to_owned(),
        Json::Text(entry.model.as_deref().unwrap_or_default().to_owned()),
    );
    if let Some(provider) = &entry.custom_llm_provider {
        params.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(provider.to_owned()),
        );
    }
    let allowed: Vec<Json> = entry
        .allowed_openai_params
        .iter()
        .map(|param| Json::Text(param.to_owned()))
        .collect();
    if !allowed.is_empty() {
        params.insert("allowed_openai_params".to_owned(), Json::List(allowed));
    }
    let mut model_info = JsonObject::new();
    if let Some(ceiling) = entry.max_output_tokens {
        model_info.insert(
            "max_output_tokens".to_owned(),
            Json::Number(Number::from(ceiling)),
        );
    }
    DeploymentSpec {
        model_name: entry.group.clone(),
        litellm_params: params,
        model_info,
        extra: JsonObject::new(),
    }
}
