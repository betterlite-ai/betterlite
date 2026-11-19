use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_capability_predicates::{CapabilityPredicates, CapabilityRequest};

// The provider's declared parameters are unknown to a case, so every option step reads them as absent.
pub(super) struct Declared;

impl SupportedParameterSource for Declared {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

// LiteLLM reads capabilities from its pinned model map, so a case never reaches a real lookup here.
pub(super) struct Unsupported;

impl CapabilityPredicates for Unsupported {
    fn supports_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
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
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}
