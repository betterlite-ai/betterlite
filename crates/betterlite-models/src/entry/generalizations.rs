use super::ModelFields;
use serde::{Deserialize, Serialize};

// Types the declaration only; matching pattern against an id is B018's catalog_generalizations.
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

// This sentinel entry is a list of rules, never a model itself (see pitfalls).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FallbackGeneralizations {
    pub rules: Vec<FallbackGeneralizationRule>,
}
