use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchOptionalParams {
    pub max_results: Option<i64>,
    pub mode: Option<String>,
    pub advanced_settings: Option<AdvancedSettings>,
    #[serde(rename = "_parallel_ai_usage")]
    pub provider_usage: Option<ProviderUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AdvancedSettings {
    pub max_results: Option<i64>,
}

// anything that is not a list of mappings fails the usage validation and reads as no usage
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ProviderUsage {
    Items(Vec<BTreeMap<String, UsageField>>),
    Invalid(IgnoredAny),
}

// only a string name and a non-negative integer count mean something to the SKU counting
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum UsageField {
    Count(u64),
    Text(String),
    Other(IgnoredAny),
}
