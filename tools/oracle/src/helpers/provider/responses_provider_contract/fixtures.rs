use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use betterlite_provider::api::responses_provider_contract::config::ResponsesProviderConfig;
use betterlite_provider::api::responses_provider_contract::error::ResponsesContractError;
use betterlite_provider::api::responses_provider_contract::records::{
    CompactRequest, CreateRequest, Headers, IdRequest, InputItemsRequest, Params, PreparedRequest,
    ResponsesLogging, ResponsesUpstream, api_base_or_required,
};
use betterlite_provider::openai_dispatch::OpenAi;
use betterlite_provider::provider::Provider;
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum FixtureError {
    #[error(transparent)]
    Contract(#[from] ResponsesContractError),
    #[error("the fixture does not answer this operation")]
    Unused,
}

impl From<FixtureError> for Box<crate::helpers::HelperError> {
    fn from(error: FixtureError) -> Self {
        Self::new(crate::helpers::HelperError {
            class: String::from("ValueError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

#[derive(Default)]
pub(super) struct Fixture {
    forwarded: Mutex<Vec<Params>>,
    complete_url_calls: Mutex<usize>,
}

impl Fixture {
    pub(super) fn forwarded(&self) -> Vec<Params> {
        self.forwarded
            .lock()
            .map_or_else(|_| Vec::new(), |calls| calls.clone())
    }

    pub(super) fn complete_url_calls(&self) -> usize {
        let Ok(calls) = self.complete_url_calls.lock() else {
            return 0;
        };
        *calls
    }
}

#[async_trait]
impl ResponsesProviderConfig for Fixture {
    type Error = FixtureError;

    fn provider(&self) -> Provider {
        Provider::OpenAi(OpenAi)
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _litellm_params: &Params,
    ) -> Result<String, FixtureError> {
        if let Ok(mut calls) = self.complete_url_calls.lock() {
            *calls = calls.saturating_add(1);
        }
        Ok(api_base_or_required(api_base)?.to_owned())
    }

    fn transform_create_request(&self, request: &CreateRequest) -> Result<Params, FixtureError> {
        let call = Params::from([
            ("model".to_owned(), Json::Text(request.model.clone())),
            ("input".to_owned(), request.input.clone()),
            (
                "response_api_optional_request_params".to_owned(),
                object(&request.optional_params),
            ),
            ("litellm_params".to_owned(), object(&request.litellm_params)),
            ("headers".to_owned(), headers_object(&request.headers)),
        ]);
        if let Ok(mut forwarded) = self.forwarded.lock() {
            forwarded.push(call);
        }
        Ok(Params::from([(
            "transformed".to_owned(),
            Json::Text(request.model.clone()),
        )]))
    }

    fn supported_params(&self, _model: &str) -> Result<Vec<String>, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn map_params(
        &self,
        _optional_params: &ResponsesApiRequestParams,
        _model: &str,
        _drop_params: bool,
    ) -> Result<Params, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_create_response(
        &self,
        _model: &str,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_streaming_response(
        &self,
        _model: &str,
        _chunk: &Json,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesAPIStreamingResponse, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_get_request(&self, _request: &IdRequest) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_get_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_delete_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_delete_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<DeleteResponseResult, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_input_items_request(
        &self,
        _request: &InputItemsRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_input_items_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<Params, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_cancel_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_cancel_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_compact_request(
        &self,
        _request: &CompactRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_compact_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, FixtureError> {
        Err(FixtureError::Unused)
    }
}

fn object(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

fn headers_object(headers: &Headers) -> Json {
    Json::Object(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}
