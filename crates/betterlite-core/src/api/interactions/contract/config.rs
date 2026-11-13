use std::error::Error as StdError;

use betterlite_interactions::types::response::{
    CancelInteractionResult, DeleteInteractionResult, InteractionsApiResponse,
    InteractionsStreamChunk,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;

use super::error::{BaseLlmException, InteractionsContractError};
use super::records::{
    CompleteUrlRequest, CreateInteractionRequest, EnvironmentRequest, InteractionIdRequest,
    InteractionsLogging, InteractionsUpstream, Params, PreparedRequest, RequestHeaders,
    ResponseHeaders,
};

pub trait BaseInteractionsAPIConfig: Send + Sync {
    type Error: StdError + Send + Sync + 'static + From<InteractionsContractError>;

    fn custom_llm_provider(&self) -> ProviderId;

    fn get_supported_params(&self, model: &str) -> Vec<String>;

    fn validate_environment(
        &self,
        request: &EnvironmentRequest<'_>,
    ) -> Result<RequestHeaders, Self::Error>;

    fn get_complete_url(&self, request: &CompleteUrlRequest<'_>) -> Result<String, Self::Error>;

    fn transform_request(
        &self,
        request: &CreateInteractionRequest<'_>,
    ) -> Result<Params, Self::Error>;

    fn transform_response(
        &self,
        model: Option<&str>,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error>;

    fn transform_streaming_response(
        &self,
        model: Option<&str>,
        parsed_chunk: &JsonValue,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsStreamChunk, Self::Error>;

    fn transform_get_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_get_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<InteractionsApiResponse, Self::Error>;

    fn transform_delete_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_delete_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
        interaction_id: &str,
    ) -> Result<DeleteInteractionResult, Self::Error>;

    fn transform_cancel_request(
        &self,
        request: &InteractionIdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_cancel_response(
        &self,
        raw_response: &InteractionsUpstream,
        logging: &mut InteractionsLogging,
    ) -> Result<CancelInteractionResult, Self::Error>;

    // LiteLLM's base get_error_class answers by raising BaseLLMException, so the value it produces is the error itself
    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &ResponseHeaders,
    ) -> Self::Error {
        Self::Error::from(InteractionsContractError::BaseLlm(BaseLlmException {
            status_code,
            message: error_message.to_owned(),
            headers: headers.clone(),
        }))
    }
}
