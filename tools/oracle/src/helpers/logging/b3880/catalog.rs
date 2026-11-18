use std::collections::BTreeSet;

use betterlite_logging::prometheus::failure::RequestedModelCatalog;

// the two models the provider cases name, with the provider LiteLLM's cost map resolves at the pin
pub(crate) const PROVIDER_CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

pub(crate) struct CaseCatalog {
    pub(crate) recognized: BTreeSet<String>,
    pub(crate) team_public: BTreeSet<String>,
    pub(crate) patterns: Vec<String>,
    pub(crate) team_patterns: Vec<String>,
}

impl RequestedModelCatalog for CaseCatalog {
    fn is_recognized_model(&self, model: &str) -> bool {
        self.recognized.contains(model)
    }

    fn is_team_public_model(&self, model: &str) -> bool {
        self.team_public.contains(model)
    }

    fn matches_pattern_route(&self, model: &str) -> bool {
        self.patterns
            .iter()
            .chain(&self.team_patterns)
            .any(|pattern| pattern_matches(pattern, model))
    }
}

pub(crate) fn boxed(input: &super::inputs::CatalogInput) -> Box<dyn RequestedModelCatalog> {
    Box::new(CaseCatalog {
        recognized: input.recognized.iter().cloned().collect(),
        team_public: input.team_public.iter().cloned().collect(),
        patterns: input.patterns.clone(),
        team_patterns: input.team_patterns.clone(),
    })
}

fn pattern_matches(pattern: &str, model: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or_else(|| pattern == model, |prefix| model.starts_with(prefix))
}
