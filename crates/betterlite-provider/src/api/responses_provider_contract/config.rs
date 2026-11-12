use std::convert::Infallible;
use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;

use crate::provider::Provider;

use super::error::{BaseLlmException, ResponsesContractError};
use super::records::{
    CompactRequest, CreateRequest, Headers, IdRequest, InputItemsRequest, Params, PreparedRequest,
    ResponsesLogging, ResponsesUpstream, SignRequest, api_base_or_required,
};

#[async_trait]
pub trait ResponsesProviderConfig: Send + Sync {
    type Error: Error + Send + Sync + 'static + From<ResponsesContractError>;

    fn provider(&self) -> Provider;

    fn supports_native_file_search(&self) -> bool {
        false
    }

    fn supports_encrypted_messages(&self) -> bool {
        false
    }

    fn supports_native_websocket(&self) -> bool {
        false
    }

    fn model_in_websocket_url(&self) -> bool {
        true
    }

    fn should_fake_stream(
        &self,
        _model: Option<&str>,
        _stream: Option<bool>,
        _custom_llm_provider: Option<&str>,
    ) -> bool {
        false
    }

    fn sign_request(&self, request: SignRequest) -> (Headers, Option<Vec<u8>>) {
        (request.headers, None)
    }

    fn supported_params(&self, model: &str) -> Result<Vec<String>, Self::Error>;

    fn map_params(
        &self,
        optional_params: &ResponsesApiRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error>;

    fn validate_environment(
        &self,
        _headers: &Headers,
        _model: &str,
        _litellm_params: Option<&Params>,
    ) -> Result<Headers, Self::Error> {
        Ok(Headers::new())
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        Ok(api_base_or_required(api_base)?.to_owned())
    }

    fn transform_create_request(&self, request: &CreateRequest) -> Result<Params, Self::Error>;

    async fn transform_create_request_async(
        &self,
        request: &CreateRequest,
    ) -> Result<Params, Self::Error> {
        self.transform_create_request(request)
    }

    fn transform_create_response(
        &self,
        model: &str,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn transform_streaming_response(
        &self,
        model: &str,
        chunk: &Json,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesAPIStreamingResponse, Self::Error>;

    fn transform_get_request(&self, request: &IdRequest) -> Result<PreparedRequest, Self::Error>;

    fn transform_get_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn transform_delete_request(&self, request: &IdRequest)
    -> Result<PreparedRequest, Self::Error>;

    fn transform_delete_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<DeleteResponseResult, Self::Error>;

    fn transform_input_items_request(
        &self,
        request: &InputItemsRequest,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_input_items_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<Params, Self::Error>;

    fn transform_cancel_request(&self, request: &IdRequest)
    -> Result<PreparedRequest, Self::Error>;

    fn transform_cancel_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn transform_compact_request(
        &self,
        request: &CompactRequest,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_compact_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: Headers,
    ) -> Result<Infallible, Self::Error> {
        Err(Self::Error::from(ResponsesContractError::BaseLlm(
            BaseLlmException {
                status_code,
                message: error_message.to_owned(),
                headers,
            },
        )))
    }

    fn transform_extra_body(
        &self,
        extra_body: &Params,
        _request: &Params,
        _model: &str,
        _litellm_params: &Params,
    ) -> Params {
        extra_body.clone()
    }

    fn websocket_url(
        &self,
        api_base: Option<&str>,
        litellm_params: &Params,
    ) -> Result<String, Self::Error> {
        let url = self.complete_url(api_base, litellm_params)?;
        Ok(url
            .replace("https://", "wss://")
            .replace("http://", "ws://"))
    }
}
