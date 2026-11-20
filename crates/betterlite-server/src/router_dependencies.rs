use serde::{Deserialize, Serialize};

pub mod config_write;
pub mod dependencies;
pub mod error;
pub mod predicates;
mod tests;
mod tier_pools;
pub mod validation;
mod values;

pub const AUTO_ROUTER_MODEL_PREFIX: &str = "auto_router/";

// LiteLLM's STRATEGY_ROUTER_PARAM_FIELDS: every litellm_params key that only an auto-router deployment reads
pub const STRATEGY_ROUTER_PARAM_FIELDS: [&str; 10] = [
    "auto_router_config",
    "auto_router_config_path",
    "auto_router_default_model",
    "auto_router_embedding_model",
    "auto_router_max_input_chars",
    "complexity_router_config",
    "complexity_router_default_model",
    "adaptive_router_config",
    "quality_router_config",
    "quality_router_default_model",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyRouterKind {
    Semantic,
    Complexity,
    Adaptive,
    Quality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyRouterDependencyRole {
    Tier,
    Default,
    Classifier,
    Embedding,
    Evaluation,
    Compactor,
}

impl StrategyRouterDependencyRole {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tier => "tier",
            Self::Default => "default",
            Self::Classifier => "classifier",
            Self::Embedding => "embedding",
            Self::Evaluation => "evaluation",
            Self::Compactor => "compactor",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrategyRouterDependency {
    pub model_name: String,
    pub role: StrategyRouterDependencyRole,
}

// a reserved name is matched by prefix, and everything else under the prefix is a semantic router
#[must_use]
pub fn classify_strategy_router_model(model: &str) -> Option<StrategyRouterKind> {
    let remainder = model.strip_prefix(AUTO_ROUTER_MODEL_PREFIX)?;
    if remainder.starts_with("complexity_router") {
        return Some(StrategyRouterKind::Complexity);
    }
    if remainder.starts_with("adaptive_router") {
        return Some(StrategyRouterKind::Adaptive);
    }
    if remainder.starts_with("quality_router") {
        return Some(StrategyRouterKind::Quality);
    }
    Some(StrategyRouterKind::Semantic)
}

#[must_use]
pub fn is_complexity_router_model(model: Option<&str>) -> bool {
    classify_strategy_router_model(model.unwrap_or_default())
        == Some(StrategyRouterKind::Complexity)
}

const SEMANTIC_REQUIRED_FIELDS: [&[&str]; 3] = [
    &["auto_router_config", "auto_router_config_path"],
    &["auto_router_default_model"],
    &["auto_router_embedding_model"],
];
const COMPLEXITY_REQUIRED_FIELDS: [&[&str]; 1] = [&[
    "complexity_router_config",
    "complexity_router_default_model",
]];
const ADAPTIVE_REQUIRED_FIELDS: [&[&str]; 1] = [&["adaptive_router_config"]];
const QUALITY_REQUIRED_FIELDS: [&[&str]; 1] =
    [&["quality_router_config", "quality_router_default_model"]];

// each group is satisfied by any one of its names, and every group must be satisfied
#[must_use]
pub(crate) const fn required_fields(
    kind: StrategyRouterKind,
) -> &'static [&'static [&'static str]] {
    match kind {
        StrategyRouterKind::Semantic => &SEMANTIC_REQUIRED_FIELDS,
        StrategyRouterKind::Complexity => &COMPLEXITY_REQUIRED_FIELDS,
        StrategyRouterKind::Adaptive => &ADAPTIVE_REQUIRED_FIELDS,
        StrategyRouterKind::Quality => &QUALITY_REQUIRED_FIELDS,
    }
}

// the complexity group flattened, so a predicate and the write check cannot drift apart
pub(crate) fn complexity_router_fields() -> impl Iterator<Item = &'static str> {
    required_fields(StrategyRouterKind::Complexity)
        .iter()
        .flat_map(|group| group.iter().copied())
}
