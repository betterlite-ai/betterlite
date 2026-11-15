use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub type Extensions = BTreeMap<String, Box<RawValue>>;

// LiteLLM echoes the whole response in the error detail, so every field and explicit null must serialize back as read
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LakeraModerationResponse {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub results: Option<Option<Vec<LakeraResult>>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LakeraResult {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub flagged: Option<Option<bool>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub category_scores: Option<Option<CategoryScores>>,
    #[serde(flatten)]
    pub extra: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryScores {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub prompt_injection: Option<Option<f64>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "serde_with::rust::double_option"
    )]
    pub jailbreak: Option<Option<f64>>,
    #[serde(flatten)]
    pub extra: Extensions,
}
