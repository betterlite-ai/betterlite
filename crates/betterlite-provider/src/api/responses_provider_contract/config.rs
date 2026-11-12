use std::collections::BTreeMap;
use std::convert::Infallible;
use std::error::Error;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use secrecy::SecretString;
use serde::Serialize;

use crate::provider::Provider;

use super::error::{BaseLlmException, ResponsesContractError};

pub type Params = BTreeMap<String, Json>;
pub type Headers = BTreeMap<String, String>;
pub type PreparedRequest = (String, Params);

const DEFAULT_INPUT_ITEMS_LIMIT: i64 = 20;

// The closed set of operations BaseResponsesAPIConfig declares; a new member changes the trait for every provider
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesOperation {
    Create,
    Stream,
    Get,
    Delete,
    InputItems,
    Cancel,
    Compact,
}

impl ResponsesOperation {
    pub const ALL: [Self; 7] = [
        Self::Create,
        Self::Stream,
        Self::Get,
        Self::Delete,
        Self::InputItems,
        Self::Cancel,
        Self::Compact,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputItemsOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InputItemsQuery<'request> {
    pub after: Option<&'request str>,
    pub before: Option<&'request str>,
    pub include: Option<&'request [String]>,
    pub limit: i64,
    pub order: InputItemsOrder,
}

impl Default for InputItemsQuery<'_> {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            include: None,
            limit: DEFAULT_INPUT_ITEMS_LIMIT,
            order: InputItemsOrder::Desc,
        }
    }
}

pub struct CreateRequest<'request> {
    pub model: &'request str,
    pub input: &'request Json,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct IdRequest<'request> {
    pub response_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct InputItemsRequest<'request> {
    pub response_id: &'request str,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
    pub query: InputItemsQuery<'request>,
}

pub struct CompactRequest<'request> {
    pub model: &'request str,
    pub input: &'request Json,
    pub optional_params: &'request Params,
    pub api_base: &'request str,
    pub litellm_params: &'request Params,
    pub headers: &'request Headers,
}

pub struct SignRequest<'request> {
    pub headers: &'request Headers,
    pub optional_params: &'request Params,
    pub request_data: &'request Params,
    pub api_base: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub model: Option<&'request str>,
    pub stream: Option<bool>,
    pub fake_stream: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponsesUpstream {
    pub status_code: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponsesLogging {
    pub model_call_details: Params,
}

pub const fn api_base_or_required(api_base: Option<&str>) -> Result<&str, ResponsesContractError> {
    let Some(base) = api_base else {
        return Err(ResponsesContractError::ApiBaseRequired);
    };
    Ok(base)
}

#[must_use]
pub fn strip_custom_call_namespace(input: &Json) -> Json {
    let Json::List(items) = input else {
        return input.clone();
    };
    Json::List(items.iter().map(strip_item_namespace).collect())
}

fn strip_item_namespace(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    let is_custom_call =
        matches!(fields.get("type"), Some(Json::Text(name)) if name == "custom_tool_call");
    if !is_custom_call {
        return item.clone();
    }
    Json::Object(
        fields
            .iter()
            .filter(|(name, _)| name.as_str() != "namespace")
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

#[must_use]
pub fn normalize_responses_request(data: &Params) -> Params {
    let Some(input) = data.get("input") else {
        return data.clone();
    };
    let mut normalized = data.clone();
    normalized.insert("input".to_owned(), strip_custom_call_namespace(input));
    normalized
}

// LiteLLM's BaseResponsesAPIConfig: providers fill the abstract transformations and inherit the rest
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

    fn sign_request(&self, request: SignRequest<'_>) -> (Headers, Option<Vec<u8>>) {
        (request.headers.clone(), None)
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

    fn transform_create_request(&self, request: &CreateRequest<'_>) -> Result<Params, Self::Error>;

    async fn transform_create_request_async(
        &self,
        request: &CreateRequest<'_>,
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

    fn transform_get_request(
        &self,
        request: &IdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_get_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn transform_delete_request(
        &self,
        request: &IdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_delete_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<DeleteResponseResult, Self::Error>;

    fn transform_input_items_request(
        &self,
        request: &InputItemsRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_input_items_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<Params, Self::Error>;

    fn transform_cancel_request(
        &self,
        request: &IdRequest<'_>,
    ) -> Result<PreparedRequest, Self::Error>;

    fn transform_cancel_response(
        &self,
        response: &ResponsesUpstream,
        logging: &mut ResponsesLogging,
    ) -> Result<ResponsesApiResponse, Self::Error>;

    fn transform_compact_request(
        &self,
        request: &CompactRequest<'_>,
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
