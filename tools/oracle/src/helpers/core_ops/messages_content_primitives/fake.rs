use std::future::{Future, ready};

use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};

// Stands in for B2221: only the listed models carry the mid-conversation flag, under the right key.
pub(super) struct FlaggedModels {
    pub(super) models: Vec<String>,
}

impl CapabilityPredicates for FlaggedModels {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let flagged = key == NamedCapability::MidConversationSystem.key()
            && self.models.iter().any(|model| model == request.model);
        ready(flagged)
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
