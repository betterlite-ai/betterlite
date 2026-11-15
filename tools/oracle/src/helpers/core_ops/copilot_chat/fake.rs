use std::future::{Future, ready};

use betterlite_core::api::copilot_chat::credentials::CopilotCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use secrecy::SecretString;

// A cached, unexpired Copilot key: no login, no refresh and no cached endpoint.
pub(super) struct StoredKey {
    token: String,
}

impl StoredKey {
    pub(super) fn new(token: &str) -> Self {
        Self {
            token: token.to_owned(),
        }
    }
}

impl CopilotCredentials for StoredKey {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(Ok(Some(SecretString::from(self.token.clone()))))
    }
}

// Stands in for B2221: only the listed models report reasoning support.
#[derive(Default)]
pub(super) struct ListedReasoningModels {
    models: Vec<String>,
}

impl ListedReasoningModels {
    pub(super) fn new(models: &[String]) -> Self {
        Self {
            models: models.to_vec(),
        }
    }
}

impl CapabilityPredicates for ListedReasoningModels {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        let listed = key == NamedCapability::Reasoning.key()
            && self.models.iter().any(|model| model == request.model);
        ready(listed)
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
