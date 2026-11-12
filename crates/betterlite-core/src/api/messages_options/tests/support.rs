use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

const MODELS: &str = r#"{
    "claude-opus-4-8": {"litellm_provider": "anthropic", "supports_speed": true, "supports_sampling_params": true},
    "claude-opus-4-7": {"litellm_provider": "anthropic", "supports_sampling_params": false},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic", "supports_sampling_params": true}
}"#;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("json value")
}

pub(super) fn object(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("json object")
}

pub(super) struct Catalogs {
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    packaged: RawCatalog,
}

impl Catalogs {
    pub(super) fn new() -> Self {
        Self {
            patterns: CandidatePatterns::new().expect("candidate patterns"),
            runtime: serde_json::from_str(MODELS).expect("runtime catalog"),
            packaged: serde_json::from_str("{}").expect("packaged catalog"),
        }
    }

    pub(super) fn sources(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.packaged,
        }
    }
}
