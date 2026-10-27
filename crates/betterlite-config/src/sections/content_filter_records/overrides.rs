use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorIntentConfig {
    #[serde(default)]
    pub competitor_intent_type: Option<String>,
    #[serde(default)]
    pub brand_self: Vec<String>,
    #[serde(default)]
    pub competitors: Vec<String>,
    #[serde(default)]
    pub competitor_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub domain_words: Vec<String>,
    #[serde(default)]
    pub route_geo_cues: Vec<String>,
    #[serde(default)]
    pub descriptor_lexicon: Vec<String>,
    #[serde(default)]
    pub indirect_competitor_patterns: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub policy: BTreeMap<String, String>,
    #[serde(default)]
    pub threshold_high: Option<f64>,
    #[serde(default)]
    pub threshold_medium: Option<f64>,
    #[serde(default)]
    pub threshold_low: Option<f64>,
    #[serde(default)]
    pub reframe_message_template: Option<String>,
    #[serde(default)]
    pub refuse_message_template: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorIntentEvidenceEntry {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub r#match: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorIntentResult {
    #[serde(default)]
    pub intent: Option<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub entities: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub signals: Vec<String>,
    #[serde(default)]
    pub action_hint: Option<String>,
    #[serde(default)]
    pub evidence: Vec<CompetitorIntentEvidenceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectSecretsConfig {
    #[serde(default)]
    pub plugins_used: Vec<DetectSecretsPlugin>,
    #[serde(default)]
    pub filters_used: Vec<DetectSecretsFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectSecretsPlugin {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base64_limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectSecretsFilter {
    #[serde(default)]
    pub name: Option<String>,
}
