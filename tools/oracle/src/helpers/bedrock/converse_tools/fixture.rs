use std::future::{Future, ready};

use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_context::anthropic_catalog_lookup::candidates::CandidatePatterns;
use betterlite_context::anthropic_catalog_lookup::error::CatalogLookupError;
use betterlite_context::anthropic_catalog_lookup::model_capability;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};

#[derive(Debug, thiserror::Error)]
pub(super) enum FixtureError {
    #[error("fixture JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("candidate patterns: {0}")]
    Patterns(#[from] CatalogLookupError),
}

pub(super) struct Fixture {
    patterns: CandidatePatterns,
    runtime: RawCatalog,
    unlisted: RawCatalog,
}

impl Fixture {
    pub(super) fn load(model_cost: &Json) -> Result<Self, FixtureError> {
        let bytes = serde_json::to_vec(model_cost)?;
        Ok(Self {
            patterns: CandidatePatterns::new()?,
            runtime: serde_json::from_slice(&bytes)?,
            unlisted: RawCatalog::new(),
        })
    }

    pub(super) const fn catalogs(&self) -> CatalogSources<'_> {
        CatalogSources {
            patterns: &self.patterns,
            runtime: &self.runtime,
            packaged: &self.unlisted,
        }
    }
}

// LiteLLM resolves these through litellm.model_cost, so the stub answers from the case catalog.
pub(super) struct CaseCapabilities<'fixture> {
    pub(super) fixture: &'fixture Fixture,
}

impl CapabilityPredicates for CaseCapabilities<'_> {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let found = model_capability(
            &self.fixture.patterns,
            &self.fixture.runtime,
            &self.fixture.unlisted,
            request.model,
            key,
        );
        ready(found == Some(true))
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
