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

use super::ResponsesRequest;
use super::config::ResponsesProviderConfig;
use super::error::{BaseLlmException, ResponsesContractError};
use super::records::{
    CompactRequest, CreateRequest, Headers, IdRequest, InputItemsOrder, InputItemsQuery,
    InputItemsRequest, Params, PreparedRequest, ResponsesLogging, ResponsesUpstream, SignRequest,
    api_base_or_required, normalize_responses_request, strip_custom_call_namespace,
};
use crate::openai_dispatch::OpenAi;
use crate::operation::OperationRequest;
use crate::provider::Provider;

#[derive(Debug, Error)]
enum FixtureError {
    #[error(transparent)]
    Contract(#[from] ResponsesContractError),
    #[error("the fixture does not answer this operation")]
    Unused,
}

macro_rules! unused_responses {
    () => {
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

        fn transform_get_response(
            &self,
            _response: &ResponsesUpstream,
            _logging: &mut ResponsesLogging,
        ) -> Result<ResponsesApiResponse, FixtureError> {
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

        fn transform_cancel_response(
            &self,
            _response: &ResponsesUpstream,
            _logging: &mut ResponsesLogging,
        ) -> Result<ResponsesApiResponse, FixtureError> {
            Err(FixtureError::Unused)
        }

        fn transform_compact_response(
            &self,
            _response: &ResponsesUpstream,
            _logging: &mut ResponsesLogging,
        ) -> Result<ResponsesApiResponse, FixtureError> {
            Err(FixtureError::Unused)
        }
    };
}

struct Defaults;

#[async_trait]
impl ResponsesProviderConfig for Defaults {
    type Error = FixtureError;

    unused_responses!();

    fn transform_create_request(&self, _request: &CreateRequest) -> Result<Params, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_get_request(&self, _request: &IdRequest) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_delete_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_input_items_request(
        &self,
        _request: &InputItemsRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_cancel_request(
        &self,
        _request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }

    fn transform_compact_request(
        &self,
        _request: &CompactRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Err(FixtureError::Unused)
    }
}

#[derive(Default)]
struct Fixture {
    forwarded: Mutex<Vec<Params>>,
    routes: Mutex<Vec<&'static str>>,
    complete_url_calls: Mutex<usize>,
}

impl Fixture {
    fn forwarded(&self) -> Vec<Params> {
        self.forwarded
            .lock()
            .map_or_else(|_| Vec::new(), |calls| calls.clone())
    }

    fn routes(&self) -> Vec<&'static str> {
        self.routes
            .lock()
            .map_or_else(|_| Vec::new(), |routes| routes.clone())
    }

    fn complete_url_calls(&self) -> usize {
        let Ok(calls) = self.complete_url_calls.lock() else {
            return 0;
        };
        *calls
    }

    fn record_route(&self, route: &'static str) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.push(route);
        }
    }

    fn prepared(&self, route: &'static str, api_base: &str) -> PreparedRequest {
        self.record_route(route);
        PreparedRequest {
            url: api_base.to_owned(),
            params: Params::new(),
        }
    }
}

#[async_trait]
impl ResponsesProviderConfig for Fixture {
    type Error = FixtureError;

    unused_responses!();

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

    fn transform_create_request(&self, request: &CreateRequest) -> Result<Params, FixtureError> {
        self.record_route("create");
        let call = Params::from([
            ("model".to_owned(), Json::Text(request.model.clone())),
            ("input".to_owned(), request.input.clone()),
            (
                "optional_params".to_owned(),
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

    fn transform_get_request(&self, request: &IdRequest) -> Result<PreparedRequest, FixtureError> {
        Ok(self.prepared("get", &request.api_base))
    }

    fn transform_delete_request(
        &self,
        request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.prepared("delete", &request.api_base))
    }

    fn transform_input_items_request(
        &self,
        request: &InputItemsRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.prepared("input_items", &request.api_base))
    }

    fn transform_cancel_request(
        &self,
        request: &IdRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.prepared("cancel", &request.api_base))
    }

    fn transform_compact_request(
        &self,
        request: &CompactRequest,
    ) -> Result<PreparedRequest, FixtureError> {
        Ok(self.prepared("compact", &request.api_base))
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
}

#[test]
fn signing_passthrough() {
    let fixture = Defaults;
    let headers = Headers::from([("Authorization".to_owned(), "Bearer test".to_owned())]);
    let optional_params = params(r#"{"stream": true}"#);
    let request_data = params(r#"{"input": "hi"}"#);
    let api_key = SecretString::from("key");
    let (signed, signed_body) = fixture.sign_request(SignRequest {
        headers: headers.clone(),
        optional_params,
        request_data: request_data.clone(),
        api_base: "https://example.test".to_owned(),
        api_key: Some(api_key),
        model: Some("m".to_owned()),
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
        model: "m".to_owned(),
        input,
        optional_params: optional_params.clone(),
        litellm_params: litellm_params.clone(),
        headers: headers.clone(),
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

fn create_record() -> CreateRequest {
    CreateRequest {
        model: "m".to_owned(),
        input: text("hi"),
        optional_params: Params::new(),
        litellm_params: Params::new(),
        headers: Headers::new(),
    }
}

fn id_record() -> IdRequest {
    IdRequest {
        response_id: "resp_a".to_owned(),
        api_base: "https://example.test".to_owned(),
        litellm_params: Params::new(),
        headers: Headers::new(),
    }
}

fn items_record() -> InputItemsRequest {
    InputItemsRequest {
        response_id: "resp_a".to_owned(),
        api_base: "https://example.test".to_owned(),
        litellm_params: Params::new(),
        headers: Headers::new(),
        query: InputItemsQuery::default(),
    }
}

fn compact_record() -> CompactRequest {
    CompactRequest {
        model: "m".to_owned(),
        input: text("hi"),
        optional_params: Params::new(),
        api_base: "https://example.test".to_owned(),
        litellm_params: Params::new(),
        headers: Headers::new(),
    }
}

fn route_responses(fixture: &Fixture, operation: &OperationRequest) {
    let OperationRequest::Responses(operation_request) = operation else {
        panic!("the operation should be a Responses request");
    };
    match operation_request {
        ResponsesRequest::Create(request) => {
            fixture
                .transform_create_request(request)
                .expect("create maps");
        }
        ResponsesRequest::Stream(request) => {
            fixture
                .transform_create_request(request)
                .expect("stream maps to create");
        }
        ResponsesRequest::Get(request) => {
            fixture.transform_get_request(request).expect("get maps");
        }
        ResponsesRequest::Delete(request) => {
            fixture
                .transform_delete_request(request)
                .expect("delete maps");
        }
        ResponsesRequest::InputItems(request) => {
            fixture
                .transform_input_items_request(request)
                .expect("input-items maps");
        }
        ResponsesRequest::Cancel(request) => {
            fixture
                .transform_cancel_request(request)
                .expect("cancel maps");
        }
        ResponsesRequest::Compact(request) => {
            fixture
                .transform_compact_request(request)
                .expect("compact maps");
        }
    }
}

#[rstest]
#[case::create(ResponsesRequest::Create(create_record()), "create")]
#[case::stream(ResponsesRequest::Stream(create_record()), "create")]
#[case::get(ResponsesRequest::Get(id_record()), "get")]
#[case::delete(ResponsesRequest::Delete(id_record()), "delete")]
#[case::input_items(ResponsesRequest::InputItems(items_record()), "input_items")]
#[case::cancel(ResponsesRequest::Cancel(id_record()), "cancel")]
#[case::compact(ResponsesRequest::Compact(compact_record()), "compact")]
fn responses_operations_map_to_their_transformations(
    #[case] request: ResponsesRequest,
    #[case] route: &'static str,
) {
    let fixture = Fixture::default();
    let operation = OperationRequest::Responses(request);
    route_responses(&fixture, &operation);
    assert_eq!(fixture.routes(), vec![route]);
}
