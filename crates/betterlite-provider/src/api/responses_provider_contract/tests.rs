#![cfg(test)]

use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::api::responses_shapes::DeleteResponseResult;
use rstest::rstest;
use secrecy::SecretString;
use thiserror::Error;

use super::config::{
    CompactRequest, CreateRequest, Headers, IdRequest, InputItemsOrder, InputItemsQuery,
    InputItemsRequest, Params, PreparedRequest, ResponsesLogging, ResponsesOperation,
    ResponsesProviderConfig, ResponsesUpstream, SignRequest, api_base_or_required,
    normalize_responses_request, strip_custom_call_namespace,
};
use super::error::{BaseLlmException, ResponsesContractError};
use crate::openai_dispatch::OpenAi;
use crate::provider::Provider;

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] ResponsesContractError),
    #[error("the fixture does not answer this operation")]
    Unused,
}

// The provider transformations the defaults never call, so every check that only exercises a default uses them
struct Defaults;

#[async_trait]
impl ResponsesProviderConfig for Defaults {
    type Error = FixtureError;

    fn provider(&self) -> Provider {
        Provider::OpenAi(OpenAi)
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

    fn transform_create_request(
        &self,
        _request: &CreateRequest<'_>,
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

    fn transform_get_request(
        &self,
        _request: &IdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
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
        _request: &IdRequest<'_>,
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
        _request: &InputItemsRequest<'_>,
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
        _request: &IdRequest<'_>,
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
        _request: &CompactRequest<'_>,
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

#[derive(Default)]
struct Fixture {
    forwarded: Mutex<Vec<Params>>,
    complete_url_calls: Mutex<usize>,
}

impl Fixture {
    fn forwarded(&self) -> Vec<Params> {
        let forwarded = self.forwarded.lock().expect("the call log lock is free");
        forwarded.clone()
    }

    fn complete_url_calls(&self) -> usize {
        let calls = self
            .complete_url_calls
            .lock()
            .expect("the call count lock is free");
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
        let base = api_base_or_required(api_base)?;
        if let Ok(mut calls) = self.complete_url_calls.lock() {
            *calls = calls.saturating_add(1);
        }
        Ok(format!("{base}/responses"))
    }

    fn transform_create_request(
        &self,
        request: &CreateRequest<'_>,
    ) -> Result<Params, FixtureError> {
        let call = Params::from([
            ("model".to_owned(), Json::Text(request.model.to_owned())),
            ("input".to_owned(), request.input.clone()),
            (
                "optional_params".to_owned(),
                object(request.optional_params),
            ),
            ("litellm_params".to_owned(), object(request.litellm_params)),
            ("headers".to_owned(), headers_object(request.headers)),
        ]);
        if let Ok(mut forwarded) = self.forwarded.lock() {
            forwarded.push(call);
        }
        Ok(Params::from([(
            "transformed".to_owned(),
            Json::Text(request.model.to_owned()),
        )]))
    }

    fn transform_input_items_request(
        &self,
        _request: &InputItemsRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
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

    fn transform_get_request(
        &self,
        _request: &IdRequest<'_>,
    ) -> Result<PreparedRequest, FixtureError> {
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
        _request: &IdRequest<'_>,
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

    fn transform_input_items_response(
        &self,
        _response: &ResponsesUpstream,
        _logging: &mut ResponsesLogging,
    ) -> Result<Params, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_cancel_request(
        &self,
        _request: &IdRequest<'_>,
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
        _request: &CompactRequest<'_>,
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

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("params should parse")
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
            .map(|(name, value)| (name.clone(), text(value)))
            .collect(),
    )
}

#[test]
fn base_capabilities() {
    let fixture = Fixture::default();
    assert_eq!(fixture.provider(), Provider::OpenAi(OpenAi));
    assert!(!fixture.supports_native_file_search());
    assert!(!fixture.supports_encrypted_messages());
    assert!(!fixture.supports_native_websocket());
    assert!(fixture.model_in_websocket_url());
    assert!(!fixture.should_fake_stream(Some("m"), Some(true), Some("openai")));
    let query = InputItemsQuery::default();
    assert_eq!(query.after, None);
    assert_eq!(query.before, None);
    assert_eq!(query.include, None);
    assert_eq!(query.limit, 20);
    assert_eq!(query.order, InputItemsOrder::Desc);
    assert_eq!(
        ResponsesOperation::ALL,
        [
            ResponsesOperation::Create,
            ResponsesOperation::Stream,
            ResponsesOperation::Get,
            ResponsesOperation::Delete,
            ResponsesOperation::InputItems,
            ResponsesOperation::Cancel,
            ResponsesOperation::Compact,
        ]
    );
}

#[test]
fn signing_passthrough() {
    let fixture = Defaults;
    let headers = Headers::from([("Authorization".to_owned(), "Bearer test".to_owned())]);
    let optional_params = params(r#"{"stream": true}"#);
    let request_data = params(r#"{"input": "hi"}"#);
    let api_key = SecretString::from("key");
    let (signed, signed_body) = fixture.sign_request(SignRequest {
        headers: &headers,
        optional_params: &optional_params,
        request_data: &request_data,
        api_base: "https://example.test",
        api_key: Some(&api_key),
        model: Some("m"),
        stream: Some(true),
        fake_stream: Some(false),
    });
    assert_eq!(signed, headers);
    assert_eq!(signed_body, None);
    let extra_body = params(r#"{"custom": 7}"#);
    assert_eq!(
        fixture.transform_extra_body(&extra_body, &request_data, "m", &Params::new()),
        extra_body
    );
    assert_eq!(
        fixture
            .validate_environment(&headers, "m", None)
            .expect("the base body returns an empty map"),
        Headers::new()
    );
}

#[rstest]
#[case::https(
    "https://example.test/responses",
    "https://example.test/responses",
    "wss://example.test/responses"
)]
#[case::http(
    "http://example.test/responses",
    "http://example.test/responses",
    "ws://example.test/responses"
)]
#[case::query(
    "http://example.test/responses?x=1",
    "http://example.test/responses?x=1",
    "ws://example.test/responses?x=1"
)]
#[case::embedded_http(
    "https://example.test/responses?next=http://inner.test/x",
    "https://example.test/responses?next=http://inner.test/x",
    "wss://example.test/responses?next=ws://inner.test/x"
)]
#[case::empty("", "", "")]
fn websocket_url(#[case] api_base: &str, #[case] expected_http: &str, #[case] expected_ws: &str) {
    let fixture = Defaults;
    let litellm_params = Params::new();
    assert_eq!(
        fixture
            .complete_url(Some(api_base), &litellm_params)
            .expect("an explicit api_base passes through"),
        expected_http
    );
    assert_eq!(
        fixture
            .websocket_url(Some(api_base), &litellm_params)
            .expect("an explicit api_base maps to a websocket scheme"),
        expected_ws
    );
}

#[test]
fn websocket_url_requires_api_base() {
    let fixture = Defaults;
    let litellm_params = Params::new();
    let error = fixture
        .complete_url(None, &litellm_params)
        .expect_err("a null api_base is refused");
    assert!(matches!(
        error,
        FixtureError::Contract(ResponsesContractError::ApiBaseRequired)
    ));
    assert_eq!(error.to_string(), "api_base is required");
    let websocket = fixture
        .websocket_url(None, &litellm_params)
        .expect_err("a null api_base is refused before the scheme changes");
    assert_eq!(websocket.to_string(), "api_base is required");
}

#[test]
fn websocket_url_uses_the_complete_url_override() {
    let fixture = Fixture::default();
    let url = fixture
        .websocket_url(Some("https://example.test"), &Params::new())
        .expect("the override answers with a websocket url");
    assert_eq!(url, "wss://example.test/responses");
    assert_eq!(fixture.complete_url_calls(), 1);
}

#[rstest]
#[case::custom_tool_call(
    r#"{"input": [{"type": "custom_tool_call", "namespace": "ns", "name": "edit", "input": "x"}]}"#,
    r#"{"input": [{"type": "custom_tool_call", "name": "edit", "input": "x"}]}"#
)]
#[case::function_call_keeps_namespace(
    r#"{"input": [{"type": "function_call", "namespace": "ns", "name": "f"}]}"#,
    r#"{"input": [{"type": "function_call", "namespace": "ns", "name": "f"}]}"#
)]
#[case::text_input(r#"{"input": "hello"}"#, r#"{"input": "hello"}"#)]
#[case::absent_input(r#"{"other": true}"#, r#"{"other": true}"#)]
#[case::unrelated_keys_survive(
    r#"{"model": "m", "input": [{"type": "custom_tool_call", "namespace": "ns", "name": "edit", "input": "x"}], "store": false}"#,
    r#"{"model": "m", "input": [{"type": "custom_tool_call", "name": "edit", "input": "x"}], "store": false}"#
)]
#[case::non_object_items(
    r#"{"input": [7, {"type": "custom_tool_call", "namespace": "ns", "name": "edit", "input": "x"}]}"#,
    r#"{"input": [7, {"type": "custom_tool_call", "name": "edit", "input": "x"}]}"#
)]
fn request_normalization(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(
        normalize_responses_request(&params(source)),
        params(expected)
    );
}

#[test]
fn strip_custom_call_namespace_leaves_other_values_alone() {
    let value = text("hello");
    assert_eq!(strip_custom_call_namespace(&value), value);
    let without_namespace = params(r#"{"type": "custom_tool_call", "name": "edit", "input": "x"}"#);
    assert_eq!(
        strip_custom_call_namespace(&object(&without_namespace)),
        object(&without_namespace)
    );
}

#[tokio::test]
async fn async_request_forwarding() {
    let fixture = Fixture::default();
    let input = text("hi");
    let optional_params = params(r#"{"store": false}"#);
    let litellm_params = params(r#"{"api_key": "k"}"#);
    let headers = Headers::from([("x-test".to_owned(), "yes".to_owned())]);
    let request = CreateRequest {
        model: "m",
        input: &input,
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let result = fixture
        .transform_create_request_async(&request)
        .await
        .expect("the asynchronous default forwards to the synchronous transform");
    assert_eq!(
        result,
        Params::from([("transformed".to_owned(), text("m"))])
    );
    assert_eq!(
        fixture.forwarded(),
        vec![Params::from([
            ("model".to_owned(), text("m")),
            ("input".to_owned(), text("hi")),
            ("optional_params".to_owned(), object(&optional_params)),
            ("litellm_params".to_owned(), object(&litellm_params)),
            ("headers".to_owned(), headers_object(&headers)),
        ])]
    );
}

#[test]
fn base_error_fields() {
    let fixture = Defaults;
    let headers = Headers::from([("retry-after".to_owned(), "3".to_owned())]);
    let error = fixture
        .error_class("bad", 429, headers.clone())
        .expect_err("the base error class raises instead of answering a value");
    let FixtureError::Contract(ResponsesContractError::BaseLlm(BaseLlmException {
        status_code,
        message,
        headers: raised_headers,
    })) = error
    else {
        panic!("the raised error should be the base exception");
    };
    assert_eq!(status_code, 429);
    assert_eq!(message, "bad");
    assert_eq!(raised_headers, headers);
}
