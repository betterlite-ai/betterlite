use super::ModelFields;
use serde::{Deserialize, Serialize};

/// One rule from the pinned catalog's `fallback_generalizations.rules`.
///
/// This is the shape `LiteLLM` uses to generalize an unmapped model id to a known family. It
/// types the declaration only; matching an id against `pattern` and applying `model_info` is
/// B018's `catalog_generalizations` engine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackGeneralizationRule {
    pub name: String,
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fill_missing_for_providers: Vec<String>,
    pub model_info: ModelFields,
}

/// The pinned catalog's `fallback_generalizations` sentinel entry: a list of
/// rules, never a model itself (see pitfalls).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackGeneralizations {
    pub rules: Vec<FallbackGeneralizationRule>,
}
