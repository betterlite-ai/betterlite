use std::collections::BTreeMap;

use betterlite_config::sections::quality_router as section;

use super::error::QualityRouterError;

// DEFAULT_COMPLEXITY_TO_QUALITY: the pinned mapping a file replaces by declaring its own table
const DEFAULT_COMPLEXITY_TO_QUALITY: [(&str, i64); 4] = [
    ("SIMPLE", 1),
    ("MEDIUM", 2),
    ("COMPLEX", 3),
    ("REASONING", 4),
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub available_models: Vec<String>,
    pub default_model: Option<String>,
    pub complexity_to_quality: BTreeMap<String, i64>,
    // keys LiteLLM accepts and ignores, kept only so the router can report them
    pub ignored: Vec<String>,
}

impl Config {
    #[must_use]
    pub fn resolve(configured: Option<&section::RouterConfig>) -> Self {
        let Some(configured) = configured else {
            return Self {
                complexity_to_quality: default_table(),
                ..Self::default()
            };
        };
        let complexity_to_quality = configured
            .complexity_to_quality
            .clone()
            .unwrap_or_else(default_table);
        Self {
            available_models: configured.available_models.clone(),
            default_model: configured.default_model.clone(),
            complexity_to_quality,
            ignored: configured.extras.keys().cloned().collect(),
        }
    }

    pub fn quality_tier(&self, complexity_name: &str) -> Result<i64, QualityRouterError> {
        self.complexity_to_quality
            .get(complexity_name)
            .copied()
            .ok_or_else(|| QualityRouterError::MissingComplexityMapping {
                tier: complexity_name.to_owned(),
            })
    }
}

fn default_table() -> BTreeMap<String, i64> {
    DEFAULT_COMPLEXITY_TO_QUALITY
        .into_iter()
        .map(|(tier, quality)| (tier.to_owned(), quality))
        .collect()
}
