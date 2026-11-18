use std::collections::BTreeMap;

use betterlite_config::sections::quality_router as section;

use super::error::QualityRouterError;

#[derive(Debug, Clone, PartialEq, Eq)]
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
                available_models: Vec::new(),
                default_model: None,
                complexity_to_quality: section::default_complexity_to_quality(),
                ignored: Vec::new(),
            };
        };
        Self {
            available_models: configured.available_models.clone(),
            default_model: configured.default_model.clone(),
            complexity_to_quality: configured.complexity_to_quality.clone(),
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
