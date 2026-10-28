use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub brand_self: Vec<String>,
    pub competitors: Vec<String>,
    pub competitor_aliases: BTreeMap<String, Vec<String>>,
    pub locations: Vec<String>,
    pub policy: BTreeMap<String, String>,
    pub threshold_high: f64,
    pub threshold_medium: f64,
    pub threshold_low: f64,
    pub reframe_message_template: Option<String>,
    pub refuse_message_template: Option<String>,
    pub comparison_words: Vec<String>,
    pub domain_words: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            brand_self: Vec::new(),
            competitors: Vec::new(),
            competitor_aliases: BTreeMap::new(),
            locations: Vec::new(),
            policy: BTreeMap::new(),
            threshold_high: 0.70,
            threshold_medium: 0.45,
            threshold_low: 0.30,
            reframe_message_template: None,
            refuse_message_template: None,
            comparison_words: [
                "better",
                "worse",
                "best",
                "vs",
                "versus",
                "compare",
                "alternative",
                "recommend",
                "ranked",
            ]
            .map(str::to_owned)
            .to_vec(),
            domain_words: Vec::new(),
        }
    }
}
