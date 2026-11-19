use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::tier_definitions::checks::ClassifierKind;

use super::values::{classifier_kind, object, text};
use super::{
    StrategyRouterDependency, StrategyRouterDependencyRole, StrategyRouterKind,
    classify_strategy_router_model,
};

const JEV_DEFAULT_MODEL: &str = "jev-latest";

// a field is a dependency only under the condition the runtime reads it, and every field is read defensively
#[must_use]
pub fn strategy_router_dependencies(litellm_params: &JsonObject) -> Vec<StrategyRouterDependency> {
    let Some(kind) =
        classify_strategy_router_model(text(litellm_params.get("model")).unwrap_or_default())
    else {
        return Vec::new();
    };
    match kind {
        StrategyRouterKind::Semantic => semantic_dependencies(litellm_params),
        StrategyRouterKind::Complexity => complexity_dependencies(litellm_params),
        StrategyRouterKind::Adaptive => pool(
            object(litellm_params.get("adaptive_router_config"))
                .and_then(|fields| fields.get("available_models")),
            StrategyRouterDependencyRole::Tier,
        ),
        StrategyRouterKind::Quality => quality_dependencies(litellm_params),
    }
}

fn semantic_dependencies(litellm_params: &JsonObject) -> Vec<StrategyRouterDependency> {
    let mut dependencies = Vec::new();
    for (key, role) in [
        (
            "auto_router_default_model",
            StrategyRouterDependencyRole::Default,
        ),
        (
            "auto_router_embedding_model",
            StrategyRouterDependencyRole::Embedding,
        ),
    ] {
        dependencies.extend(named(litellm_params.get(key), role));
    }
    dependencies
}

fn quality_dependencies(litellm_params: &JsonObject) -> Vec<StrategyRouterDependency> {
    let quality = object(litellm_params.get("quality_router_config"));
    // an absent or null deployment default takes the config default_model; a written value stays as given
    let declared = litellm_params.get("quality_router_default_model");
    let default_model = match declared {
        None | Some(Json::Null) => quality.and_then(|fields| fields.get("default_model")),
        Some(_) => declared,
    };
    let available = pool(
        quality.and_then(|fields| fields.get("available_models")),
        StrategyRouterDependencyRole::Tier,
    );
    let default = named(default_model, StrategyRouterDependencyRole::Default);
    deduplicated(available.into_iter().chain(default).collect())
}

fn complexity_dependencies(litellm_params: &JsonObject) -> Vec<StrategyRouterDependency> {
    let config = object(litellm_params.get("complexity_router_config"));
    let mut dependencies = tier_dependencies(config);
    dependencies.extend(named(
        litellm_params.get("complexity_router_default_model"),
        StrategyRouterDependencyRole::Default,
    ));
    let compaction = object(config.and_then(|fields| fields.get("context_compaction")));
    dependencies.extend(named(
        compaction.and_then(|fields| fields.get("model")),
        StrategyRouterDependencyRole::Compactor,
    ));
    let classifier = classifier_kind(config);
    if classifier.is_some_and(ClassifierKind::uses_llm) {
        let llm_config = object(config.and_then(|fields| fields.get("classifier_llm_config")));
        dependencies.extend(named(
            llm_config.and_then(|fields| fields.get("model")),
            StrategyRouterDependencyRole::Classifier,
        ));
    }
    if classifier == Some(ClassifierKind::Jev) {
        dependencies.push(jev_dependency(config));
    }
    if matches!(
        config.and_then(|fields| fields.get("semantic_keyword_matching")),
        Some(Json::Bool(true))
    ) {
        dependencies.extend(named(
            config.and_then(|fields| fields.get("embedding_model")),
            StrategyRouterDependencyRole::Embedding,
        ));
    }
    deduplicated(dependencies)
}

fn tier_dependencies(config: Option<&JsonObject>) -> Vec<StrategyRouterDependency> {
    let Some(tiers) = object(config.and_then(|fields| fields.get("tiers"))) else {
        return Vec::new();
    };
    tiers
        .values()
        .flat_map(|tier| pool(Some(tier), StrategyRouterDependencyRole::Tier))
        .collect()
}

// without a deployment_name the jev classifier calls the bundled evaluation model through typesafe/
fn jev_dependency(config: Option<&JsonObject>) -> StrategyRouterDependency {
    let jev = object(config.and_then(|fields| fields.get("jev_classifier_config")));
    let deployment =
        text(jev.and_then(|fields| fields.get("deployment_name"))).filter(|name| !name.is_empty());
    if let Some(name) = deployment {
        return StrategyRouterDependency {
            model_name: name.to_owned(),
            role: StrategyRouterDependencyRole::Classifier,
        };
    }
    let model = text(jev.and_then(|fields| fields.get("model"))).unwrap_or(JEV_DEFAULT_MODEL);
    StrategyRouterDependency {
        model_name: format!("typesafe/{model}"),
        role: StrategyRouterDependencyRole::Evaluation,
    }
}

fn named(
    value: Option<&Json>,
    role: StrategyRouterDependencyRole,
) -> Option<StrategyRouterDependency> {
    let model_name = text(value)?;
    if model_name.is_empty() {
        return None;
    }
    Some(StrategyRouterDependency {
        model_name: model_name.to_owned(),
        role,
    })
}

fn pool(value: Option<&Json>, role: StrategyRouterDependencyRole) -> Vec<StrategyRouterDependency> {
    match value {
        Some(Json::Text(_)) => named(value, role).into_iter().collect(),
        Some(Json::List(items)) => items
            .iter()
            .filter_map(|item| named(Some(item), role))
            .collect(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_)) | None => Vec::new(),
    }
}

// LiteLLM dedupes with dict.fromkeys, so the first spelling of each name and role pair wins
fn deduplicated(dependencies: Vec<StrategyRouterDependency>) -> Vec<StrategyRouterDependency> {
    let mut seen = BTreeSet::new();
    dependencies
        .into_iter()
        .filter(|dependency| seen.insert((dependency.model_name.clone(), dependency.role)))
        .collect()
}
