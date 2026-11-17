use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};
use indexmap::IndexMap;
use tokio::runtime::{Builder, Runtime};

pub(crate) const MODEL: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";
pub(crate) const NOVA: &str = "amazon.nova-pro-v1:0";
pub(crate) const LLAMA: &str = "meta.llama3-3-70b-instruct-v1:0";

pub(crate) const CATALOG: &str = r#"{
    "anthropic.claude-sonnet-4-5-20250929-v1:0": {"supports_forced_tool_use": true},
    "amazon.nova-pro-v1:0": {"supports_forced_tool_use": false},
    "meta.llama3-3-70b-instruct-v1:0": {"supports_forced_tool_use": true}
}"#;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

pub(crate) fn object(text: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(text) else {
        panic!("the fixture is an object");
    };
    fields
}

pub(crate) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

pub(crate) struct Fixture {
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(crate) fn new(model_cost: &Json) -> Self {
        let bytes = serde_json::to_vec(model_cost).expect("the model cost map serializes");
        Self {
            patterns: CandidatePatterns::new().expect("the candidate patterns compile"),
            runtime: serde_json::from_slice(&bytes).expect("the model cost map is a catalog"),
            unlisted: RawCatalog::new(),
        }
    }

    pub(crate) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Query {
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) key: String,
}

#[derive(Default)]
pub(crate) struct Predicates {
    supported: Vec<String>,
    pub(crate) queries: Vec<Query>,
}

impl Predicates {
    pub(crate) fn with(supported: &[&'static str]) -> Self {
        Self {
            supported: supported.iter().copied().map(str::to_owned).collect(),
            queries: Vec::new(),
        }
    }
}

impl CapabilityPredicates for Predicates {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.queries.push(Query {
            model: request.model.to_owned(),
            provider: request.custom_llm_provider.map(str::to_owned),
            key: key.to_owned(),
        });
        let supported = self.supported.contains(&key.to_owned());
        ready(supported)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = Json> + Send {
        ready(Json::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}
