use std::future::{Future, ready};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};

// CapabilityPredicates takes &mut self and returns impl Future, so it cannot be stored as a trait object; this look-up is the injected, object-safe form SyncManager holds
#[async_trait]
pub trait CapabilityLookup: Send + Sync {
    async fn supports(&self, model: &str, provider: Option<&str>, key: &str) -> bool;
}

// the ROI estimator asks only supports_factory with the none-reasoning key, so the other answers are the ones LiteLLM's map gives for a model it does not list
pub(crate) struct SessionCapabilities<'lookup> {
    pub(crate) lookup: &'lookup dyn CapabilityLookup,
}

impl CapabilityPredicates for SessionCapabilities<'_> {
    async fn supports_factory(&mut self, request: &CapabilityRequest<'_>, key: &str) -> bool {
        self.lookup
            .supports(request.model, request.custom_llm_provider, key)
            .await
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
