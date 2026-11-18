use std::collections::{BTreeMap, BTreeSet};

use betterlite_logging::prometheus::failure::RequestedModelCatalog;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

use super::inputs::CatalogInput;

// the two models the provider cases name, with the provider LiteLLM's cost map resolves at the pin
const PROVIDER_CATALOG: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai"},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

// the ambient get_llm_provider reads: an empty configured-name, custom-provider and env set
pub(crate) struct ProviderFixture {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl ProviderFixture {
    pub(crate) fn new() -> Result<Self, Box<HelperError>> {
        Ok(Self {
            catalog: ModelCatalog::parse(PROVIDER_CATALOG.as_bytes(), UnknownFields::Reject)
                .or_config_error()?,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        })
    }

    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

// the proxy router a case installs: which names it recognizes, which teams made public, which patterns route
pub(crate) struct CaseCatalog {
    recognized: BTreeSet<String>,
    team_public: BTreeSet<String>,
    patterns: Vec<String>,
    team_patterns: Vec<String>,
}

impl CaseCatalog {
    pub(crate) fn new(input: &CatalogInput) -> Self {
        Self {
            recognized: input.recognized.iter().cloned().collect(),
            team_public: input.team_public.iter().cloned().collect(),
            patterns: input.patterns.clone(),
            team_patterns: input.team_patterns.clone(),
        }
    }
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

// a router pattern: a trailing * matches a prefix, any other pattern names one model
fn pattern_matches(pattern: &str, model: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => model.starts_with(prefix),
        None => pattern == model,
    }
}
