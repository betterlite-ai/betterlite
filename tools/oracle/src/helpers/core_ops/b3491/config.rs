use betterlite_core::api::interactions::contract::config::BaseInteractionsAPIConfig;
use betterlite_core::api::interactions::contract::error::InteractionsContractError;
use betterlite_core::api::interactions::contract::records::{
    CompleteUrlRequest, CreateInteractionRequest, EnvironmentRequest, InteractionIdRequest,
    InteractionsLogging, InteractionsUpstream, Params, PreparedRequest, RequestHeaders,
};
use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum FixtureConfigError {
    #[error("the fixture does not answer this call")]
    Unsupported,
    #[error("the fixture chunk does not decode")]
    Decode(#[source] serde_json::Error),
    #[error("the fixture chunk does not encode")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] InteractionsContractError),
}

pub(crate) struct FixtureConfig;

impl BaseInteractionsAPIConfig for FixtureConfig {
    type Error = FixtureConfigError;

    fn custom_llm_provider(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn get_supported_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error> {
        Ok(request.headers.clone())
    }

    fn get_complete_url(&self, _request: &CompleteUrlRequest<'_>) -> Result<String, Self::Error> {
        Ok(String::new())
    }

    fn transform_request(
        &self,
        _request: &CreateInteractionRequest<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(Params::new())
    }

    fn transform_response(
        &self,
        _model: Option<&str>,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_streaming_response(
        &self,
        _model: Option<&str>,
        parsed_chunk: &JsonValue,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, Self::Error> {
        let encoded = serde_json::to_string(parsed_chunk).map_err(FixtureConfigError::Encode)?;
        let chunk = serde_json::from_str(&encoded).map_err(FixtureConfigError::Decode)?;
        Ok(chunk)
    }

    fn transform_get_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_get_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_delete_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
        _interaction_id: &str,
    ) -> Result<DeleteInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_request(
        &self,
        _request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }

    fn transform_cancel_response(
        &self,
        _raw_response: &InteractionsUpstream,
        _logging: &mut InteractionsLogging,
    ) -> Result<CancelInteractionResult, Self::Error> {
        Err(FixtureConfigError::Unsupported)
    }
}
