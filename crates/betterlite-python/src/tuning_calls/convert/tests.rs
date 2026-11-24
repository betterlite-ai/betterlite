#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::resource_sdk_settings::Raw;
use betterlite_core::api::fine_tuning_cancel_call::{
    AzureFineTuningCancel, FineTuningCancelError, FineTuningCancelValues, OpenAiFineTuningCancel,
};
use betterlite_core::api::fine_tuning_create::{
    AzureCreateCall, FineTuningCreateError, OpenAiCreateCall,
};
use betterlite_core::api::fine_tuning_read::{
    AzureReadCall, FineTuningReadError, OpenAiReadCall, ReadOperation, ReadValues,
};
use betterlite_core::api::resource_sdk_settings::timeout::{
    FineTuningTimeout, StructuredTimeout, TimeoutError,
};
use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::fine_tuning::error::FineTuneError;
use betterlite_openai::fine_tuning::job::FineTuneCreate;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::super::error::{Failure, Raised, WireError};
use super::super::register as register_tuning;
use super::SettingsWire;
use super::failures::{TuningFailure, class_for, provider_failure, response_headers, wire_failure};
use super::requests::{WireCall, create_body, deadline, request_for};
use crate::completion::register as register_completion;

fn openai_timeout() -> FineTuningTimeout {
    FineTuningTimeout::Seconds(30.0)
}

fn data() -> FineTuneCreate {
    serde_json::from_str(
        r#"{"model":"gpt-4o-mini","training_file":"file-abc","hyperparameters":{"n_epochs":2},"seed":0,"suffix":"trial"}"#,
    )
    .expect("the sample record parses")
}

fn request_of(call: WireCall<'_>) -> OutboundRequest {
    request_for(call, None).expect("the request builds").0
}

fn flag_of(call: WireCall<'_>) -> bool {
    request_for(call, None).expect("the request builds").1
}

fn openai_create() -> OpenAiCreateCall {
    OpenAiCreateCall {
        is_async: false,
        data: data(),
        api_base: "http://127.0.0.1:9/v1/".to_owned(),
        api_key: Some(SecretString::from("sk-test")),
        api_version: None,
        organization: Some("org-1".to_owned()),
        timeout: openai_timeout(),
        max_retries: None,
    }
}

fn azure_create() -> AzureCreateCall {
    AzureCreateCall {
        is_async: false,
        data: data(),
        api_base: Some("https://example.openai.azure.com".to_owned()),
        api_key: Some(SecretString::from("sk-azure")),
        api_version: None,
        organization: Some("org-1".to_owned()),
        timeout: openai_timeout(),
        max_retries: None,
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn json_body(request: &OutboundRequest) -> Json {
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the request carries a JSON body")
    };
    serde_json::from_slice(bytes).expect("the request body is JSON")
}

fn body_of(bytes: &[u8]) -> Json {
    serde_json::from_slice(bytes).expect("the request body is JSON")
}

fn field<'body>(body: &'body Json, name: &str) -> &'body Json {
    let Json::Object(fields) = body else {
        panic!("the body is not an object")
    };
    fields.get(name).expect("the body holds the field")
}

fn failure_of(raised: Raised) -> Failure {
    match raised {
        Raised::Failure(failure) => *failure,
        Raised::Python(error) => panic!("expected a LiteLLM failure, got {error}"),
    }
}

fn json_error() -> serde_json::Error {
    serde_json::from_str::<Json>("{").expect_err("the text is not JSON")
}

fn python_error(value: Raised) -> PyErr {
    let Raised::Python(error) = value else {
        panic!("the failure is a Python error");
    };
    *error
}

#[test]
fn request_openai_create_writes_the_sdk_wire() {
    let request = request_of(WireCall::OpenAiCreate(&openai_create()));
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "http://127.0.0.1:9/v1/fine_tuning/jobs"
    );
    assert!(request.query.is_empty());
    assert_eq!(
        header(&request.headers, "authorization").as_deref(),
        Some("Bearer sk-test")
    );
    assert_eq!(
        header(&request.headers, "openai-organization").as_deref(),
        Some("org-1")
    );
    assert_eq!(
        header(&request.headers, "content-type").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
    assert_eq!(request.deadline, Duration::from_secs(30));
    assert_eq!(
        json_body(&request),
        serde_json::from_str::<Json>(
            r#"{"model":"gpt-4o-mini","training_file":"file-abc","hyperparameters":{"n_epochs":2},"seed":0,"suffix":"trial"}"#
        )
        .unwrap()
    );
}

#[test]
fn request_create_body_flattens_extra_body() {
    let mut record = data();
    record.extra.insert(
        "extra_body".to_owned(),
        serde_json::from_str(r#"{"trainingType": 5}"#).expect("the extra body parses"),
    );
    let body = body_of(&create_body(&record, false, None).unwrap());
    assert_eq!(
        field(&body, "trainingType"),
        &serde_json::from_str::<Json>("5").unwrap()
    );
    let Json::Object(fields) = &body else {
        panic!("the body is not an object")
    };
    assert!(!fields.contains_key("extra_body"));
}

#[test]
fn request_create_body_keeps_the_integration_texts() {
    let integrations = vec![
        Json::Text("wandb".to_owned()),
        Json::Text("tensorboard".to_owned()),
    ];
    let body = body_of(&create_body(&data(), false, Some(&integrations)).unwrap());
    let Json::Object(fields) = &body else {
        panic!("the body is not an object")
    };
    assert_eq!(fields.get("integrations"), Some(&Json::List(integrations)));
}

#[test]
fn request_extra_body_overwrites_the_integrations_field() {
    let mut record = data();
    record.extra.insert(
        "extra_body".to_owned(),
        serde_json::from_str(r#"{"integrations": []}"#).expect("the extra body parses"),
    );
    let integrations = vec![Json::Text("wandb".to_owned())];
    let body = body_of(&create_body(&record, false, Some(&integrations)).unwrap());
    let Json::Object(fields) = &body else {
        panic!("the body is not an object")
    };
    assert_eq!(fields.get("integrations"), Some(&Json::List(Vec::new())));
}

#[test]
fn request_azure_create_defaults_the_training_type() {
    let body = body_of(&create_body(&data(), true, None).unwrap());
    assert_eq!(
        field(&body, "trainingType"),
        &serde_json::from_str::<Json>("1").unwrap()
    );
}

#[test]
fn request_azure_create_uses_the_api_key_header() {
    let request = request_of(WireCall::AzureCreate(&azure_create()));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/fine_tuning/jobs"
    );
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), "2025-02-01-preview".to_owned())]
    );
    assert_eq!(
        header(&request.headers, "api-key").as_deref(),
        Some("sk-azure")
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
    assert_eq!(header(&request.headers, "authorization"), None);
}

#[test]
fn request_azure_create_v1_uses_a_bearer_token() {
    let mut call = azure_create();
    call.api_version = Some("v1".to_owned());
    let request = request_of(WireCall::AzureCreate(&call));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/v1/fine_tuning/jobs"
    );
    assert!(request.query.is_empty());
    assert_eq!(
        header(&request.headers, "authorization").as_deref(),
        Some("Bearer sk-azure")
    );
    assert_eq!(header(&request.headers, "api-key"), None);
}

#[test]
fn request_azure_create_keeps_a_deployments_base() {
    let mut call = azure_create();
    call.api_base = Some("https://example.openai.azure.com/openai/deployments/deploy".to_owned());
    let request = request_of(WireCall::AzureCreate(&call));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/deploy/fine_tuning/jobs"
    );
}

#[test]
fn request_azure_create_reads_an_empty_version_as_absent() {
    let mut call = azure_create();
    call.api_version = Some(String::new());
    let request = request_of(WireCall::AzureCreate(&call));
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), "2025-02-01-preview".to_owned())]
    );
}

#[test]
fn request_list_sends_the_cursor_and_a_zero_limit() {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let call = OpenAiReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: Some("cursor"),
                limit: Some(0),
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let request = request_of(WireCall::OpenAiRead(&call));
    assert_eq!(request.method, Method::GET);
    assert_eq!(
        request.url.as_str(),
        "http://127.0.0.1:9/v1/fine_tuning/jobs"
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
    assert_eq!(
        request.query,
        vec![
            ("after".to_owned(), "cursor".to_owned()),
            ("limit".to_owned(), "0".to_owned()),
        ]
    );
}

#[test]
fn request_list_drops_an_empty_cursor() {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let call = OpenAiReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: Some(""),
                limit: None,
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let request = request_of(WireCall::OpenAiRead(&call));
    assert!(request.query.is_empty());
}

#[test]
fn request_retrieve_appends_the_job_id() {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let call = OpenAiReadCall {
        values: ReadValues {
            is_async: true,
            operation: ReadOperation::Retrieve { job_id: "ftjob-1" },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: Some("org-1"),
    };
    let request = request_of(WireCall::OpenAiRead(&call));
    assert_eq!(
        request.url.as_str(),
        "http://127.0.0.1:9/v1/fine_tuning/jobs/ftjob-1"
    );
    assert!(request.query.is_empty());
    assert_eq!(
        header(&request.headers, "openai-organization").as_deref(),
        Some("org-1")
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
}

#[derive(Clone, Copy)]
enum Route {
    Retrieve,
    Cancel,
}

#[rstest]
#[case::retrieve_plain(Route::Retrieve, "ftjob-1", Some("ftjob-1"))]
#[case::retrieve_slash(Route::Retrieve, "a/b", Some("a%2Fb"))]
#[case::retrieve_question(Route::Retrieve, "a?b=c", Some("a%3Fb%3Dc"))]
#[case::retrieve_empty(Route::Retrieve, "", None)]
#[case::retrieve_dot(Route::Retrieve, ".", None)]
#[case::retrieve_dot_dot(Route::Retrieve, "..", None)]
#[case::cancel_plain(Route::Cancel, "ftjob-1", Some("ftjob-1/cancel"))]
#[case::cancel_slash(Route::Cancel, "a/b", Some("a%2Fb/cancel"))]
#[case::cancel_question(Route::Cancel, "a?b=c", Some("a%3Fb%3Dc/cancel"))]
#[case::cancel_empty(Route::Cancel, "", None)]
#[case::cancel_dot(Route::Cancel, ".", None)]
#[case::cancel_dot_dot(Route::Cancel, "..", None)]
fn request_quotes_the_job_id_as_one_path_segment(
    #[case] route: Route,
    #[case] job_id: &str,
    #[case] tail: Option<&str>,
) {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let read_call = OpenAiReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::Retrieve { job_id },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let cancel_call = OpenAiFineTuningCancel {
        values: FineTuningCancelValues {
            is_async: false,
            job_id,
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: None,
        client: None,
    };
    let call = match route {
        Route::Retrieve => WireCall::OpenAiRead(&read_call),
        Route::Cancel => WireCall::OpenAiCancel(&cancel_call),
    };
    let Some(suffix) = tail else {
        let error = request_for(call, None).expect_err("the id cannot be one path segment");
        assert!(matches!(error, WireError::JobId(_)));
        return;
    };
    let request = request_of(call);
    let expected = format!("http://127.0.0.1:9/v1/fine_tuning/jobs/{suffix}");
    assert_eq!(request.url.as_str(), expected.as_str());
    assert!(request.query.is_empty());
}

#[test]
fn request_azure_list_names_the_api_version() {
    let key = SecretString::from("sk-azure");
    let timeout = openai_timeout();
    let call = AzureReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: None,
                limit: Some(5),
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: Some("https://example.openai.azure.com"),
        api_key: Some(&key),
        api_version: None,
        organization: Some("org-1"),
    };
    let request = request_of(WireCall::AzureRead(&call));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/fine_tuning/jobs"
    );
    assert_eq!(
        request.query,
        vec![
            ("api-version".to_owned(), "2025-02-01-preview".to_owned()),
            ("limit".to_owned(), "5".to_owned()),
        ]
    );
    assert_eq!(
        header(&request.headers, "api-key").as_deref(),
        Some("sk-azure")
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
}

#[test]
fn request_cancel_posts_without_a_body() {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let call = OpenAiFineTuningCancel {
        values: FineTuningCancelValues {
            is_async: false,
            job_id: "ftjob-1",
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: Some("org-1"),
        client: None,
    };
    let request = request_of(WireCall::OpenAiCancel(&call));
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url.as_str(),
        "http://127.0.0.1:9/v1/fine_tuning/jobs/ftjob-1/cancel"
    );
    assert!(request.query.is_empty());
    assert!(matches!(request.body, RequestBody::Empty));
    assert_eq!(
        header(&request.headers, "authorization").as_deref(),
        Some("Bearer sk-test")
    );
    assert_eq!(
        header(&request.headers, "accept").as_deref(),
        Some("application/json")
    );
}

#[test]
fn request_azure_cancel_quotes_the_job_id() {
    let key = SecretString::from("sk-azure");
    let timeout = openai_timeout();
    let call = AzureFineTuningCancel {
        values: FineTuningCancelValues {
            is_async: false,
            job_id: "a/b",
            timeout: &timeout,
            max_retries: None,
        },
        api_base: Some("https://example.openai.azure.com"),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let request = request_of(WireCall::AzureCancel(&call));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/fine_tuning/jobs/a%2Fb/cancel"
    );
}

#[test]
fn request_cancel_keeps_the_structured_read_timeout() {
    let key = SecretString::from("sk-azure");
    let timeout = FineTuningTimeout::Structured(StructuredTimeout {
        connect: Some(5.0_f64),
        read: Some(15.0_f64),
        write: Some(25.0_f64),
        pool: None,
    });
    let call = AzureFineTuningCancel {
        values: FineTuningCancelValues {
            is_async: true,
            job_id: "ftjob-1",
            timeout: &timeout,
            max_retries: None,
        },
        api_base: Some("https://example.openai.azure.com"),
        api_key: Some(&key),
        api_version: Some("2025-02-01-preview"),
        organization: None,
    };
    let request = request_of(WireCall::AzureCancel(&call));
    assert_eq!(request.deadline, Duration::from_secs(15));
    assert_eq!(
        request.query,
        vec![("api-version".to_owned(), "2025-02-01-preview".to_owned())]
    );
}

#[test]
fn request_a_zero_timeout_falls_back_to_ten_minutes() {
    assert_eq!(
        deadline(&FineTuningTimeout::Seconds(0.0)),
        Duration::from_secs(600)
    );
    let only_connect = FineTuningTimeout::Structured(StructuredTimeout {
        connect: Some(5.0_f64),
        read: None,
        write: None,
        pool: None,
    });
    assert_eq!(deadline(&only_connect), Duration::from_secs(5));
}

#[test]
fn request_settings_take_the_environment() {
    let wire: SettingsWire =
        serde_json::from_str(r#"{"api_base":"http://call/v1"}"#).expect("the settings parse");
    let mut environment = BTreeMap::new();
    environment.insert("OPENAI_API_KEY".to_owned(), "sk-env".to_owned());
    environment.insert("OPENAI_BASE_URL".to_owned(), "http://env/v1".to_owned());
    environment.insert("AZURE_API_VERSION".to_owned(), String::new());
    let settings = Raw { settings: wire.raw }.convert(&environment).unwrap();
    assert_eq!(settings.api_base.as_deref(), Some("http://call/v1"));
    assert_eq!(settings.openai_base_url.as_deref(), Some("http://env/v1"));
    assert_eq!(settings.azure_api_version.as_deref(), Some(""));
    assert_eq!(
        settings
            .openai_api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("sk-env".to_owned())
    );
}

#[test]
fn request_create_route_returns_the_azure_flag() {
    let openai = openai_create();
    assert!(!flag_of(WireCall::OpenAiCreate(&openai)));
    assert_eq!(
        request_of(WireCall::OpenAiCreate(&openai)).method,
        Method::POST
    );
    let azure = azure_create();
    assert!(flag_of(WireCall::AzureCreate(&azure)));
    let request = request_of(WireCall::AzureCreate(&azure));
    assert_eq!(
        header(&request.headers, "api-key").as_deref(),
        Some("sk-azure")
    );
}

#[test]
fn request_read_route_names_the_azure_flag() {
    let key = SecretString::from("sk-test");
    let timeout = openai_timeout();
    let openai = OpenAiReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: None,
                limit: None,
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    assert!(!flag_of(WireCall::OpenAiRead(&openai)));
    let azure = AzureReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: None,
                limit: None,
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: Some("https://example.openai.azure.com"),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let request = request_of(WireCall::AzureRead(&azure));
    assert!(flag_of(WireCall::AzureRead(&azure)));
    assert_eq!(request.method, Method::GET);
}

#[test]
fn request_cancel_route_builds_the_azure_url() {
    let key = SecretString::from("sk-azure");
    let timeout = openai_timeout();
    let azure = AzureFineTuningCancel {
        values: FineTuningCancelValues {
            is_async: false,
            job_id: "ftjob-1",
            timeout: &timeout,
            max_retries: None,
        },
        api_base: Some("https://example.openai.azure.com"),
        api_key: Some(&key),
        api_version: None,
        organization: None,
    };
    let request = request_of(WireCall::AzureCancel(&azure));
    assert!(flag_of(WireCall::AzureCancel(&azure)));
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/fine_tuning/jobs/ftjob-1/cancel"
    );
}

#[test]
fn request_response_headers_keep_the_text_values() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_static("abc"),
    );
    headers.insert(
        HeaderName::from_static("retry-after"),
        HeaderValue::from_static("1"),
    );
    let kept = response_headers(&headers);
    assert_eq!(kept.get("x-request-id").map(String::as_str), Some("abc"));
    assert_eq!(kept.get("retry-after").map(String::as_str), Some("1"));
}

#[test]
fn request_register_adds_the_family() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register_completion(&module).unwrap();
        register_tuning(&module).unwrap();
        assert!(module.hasattr("NativeTuningSession").unwrap());
        assert!(module.hasattr("NativeTuningError").unwrap());
        assert!(module.hasattr("NativeError").unwrap());
    });
}

#[test]
fn error_provider_status_keeps_the_class_and_the_body() {
    let body = serde_json::from_str::<Json>(
        r#"{"error":{"message":"slow down","type":"rate_limit","param":null,"code":"429"}}"#,
    )
    .unwrap();
    let mut headers = BTreeMap::new();
    headers.insert("retry-after".to_owned(), "1".to_owned());
    let failure = provider_failure(429, body.clone(), headers.clone());
    assert_eq!(failure.kind, "RateLimitError");
    assert_eq!(failure.status_code, Some(429));
    assert_eq!(failure.message, "slow down");
    assert_eq!(failure.body, Some(body));
    assert_eq!(failure.headers, headers);
}

#[test]
fn error_a_body_without_a_message_keeps_an_empty_one() {
    let failure = provider_failure(500, Json::Text("oops".to_owned()), BTreeMap::new());
    assert_eq!(failure.kind, "InternalServerError");
    assert_eq!(failure.message, "");
    assert_eq!(failure.body, Some(Json::Text("oops".to_owned())));
}

#[test]
fn error_conflict_and_unknown_statuses_are_api_errors() {
    assert_eq!(class_for(400), "BadRequestError");
    assert_eq!(class_for(401), "AuthenticationError");
    assert_eq!(class_for(403), "PermissionDeniedError");
    assert_eq!(class_for(404), "NotFoundError");
    assert_eq!(class_for(409), "APIError");
    assert_eq!(class_for(422), "UnprocessableEntityError");
    assert_eq!(class_for(429), "RateLimitError");
    assert_eq!(class_for(500), "InternalServerError");
    assert_eq!(class_for(503), "InternalServerError");
    assert_eq!(class_for(418), "APIError");
}

#[test]
fn error_unsupported_provider_is_a_bad_request() {
    let raised = Raised::from(TuningFailure::Create(FineTuningCreateError::Unsupported {
        provider: "bedrock".to_owned(),
    }));
    let failure = failure_of(raised);
    assert_eq!(failure.kind, "BadRequestError");
    assert_eq!(failure.status_code, Some(400));
    assert_eq!(failure.model.as_deref(), Some("n/a"));
    assert_eq!(failure.llm_provider.as_deref(), Some("bedrock"));
    assert_eq!(
        failure.message,
        "LiteLLM doesn't support bedrock for 'create_batch'. Only 'openai' is supported."
    );
    assert_eq!(
        failure.body,
        Some(Json::Text("Unsupported provider".to_owned()))
    );
}

#[test]
fn error_missing_openai_key() {
    let mut call = openai_create();
    call.api_key = None;
    let error =
        request_for(WireCall::OpenAiCreate(&call), None).expect_err("no key means no request");
    assert!(matches!(error, WireError::MissingOpenAiKey));
}

#[test]
fn error_missing_azure_base() {
    let mut call = azure_create();
    call.api_base = None;
    let error =
        request_for(WireCall::AzureCreate(&call), None).expect_err("no base means no request");
    assert!(matches!(error, WireError::MissingAzureBase));
}

#[test]
fn error_missing_azure_key() {
    let mut call = azure_create();
    call.api_key = None;
    let error =
        request_for(WireCall::AzureCreate(&call), None).expect_err("no key means no request");
    assert!(matches!(error, WireError::MissingAzureKey));
}

#[test]
fn error_missing_credentials_name_the_environment_variable() {
    let failure = failure_of(wire_failure(WireError::MissingOpenAiKey));
    assert_eq!(failure.kind, "Exception");
    assert!(failure.message.starts_with("Missing credentials."));
    assert!(failure.message.contains("OPENAI_API_KEY"));
    assert!(failure.message.contains("OPENAI_ADMIN_KEY"));
    let azure_failure = failure_of(wire_failure(WireError::MissingAzureKey));
    assert_eq!(azure_failure.kind, "Exception");
    assert!(azure_failure.message.starts_with("Missing credentials."));
    assert!(azure_failure.message.contains("AZURE_OPENAI_API_KEY"));
}

#[test]
fn error_a_decode_failure_is_a_validation_error() {
    let failure = failure_of(wire_failure(WireError::Decode {
        source: FineTuneError::Decode {
            source: json_error(),
        },
    }));
    assert_eq!(failure.kind, "APIResponseValidationError");
}

#[test]
fn error_a_transport_failure_is_a_connection_error() {
    let failure = failure_of(wire_failure(WireError::Transport {
        source: Box::new(TransportError::Closed),
    }));
    assert_eq!(failure.kind, "APIConnectionError");
}

#[test]
fn error_a_missing_azure_base_is_a_python_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let base = python_error(wire_failure(WireError::MissingAzureBase));
        assert!(base.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn error_a_job_id_failure_is_a_python_value_error() {
    Python::initialize();
    Python::attach(|py| {
        let dot = python_error(wire_failure(WireError::JobId(HttpError::DotSegment {
            field_name: "job_id".to_owned(),
        })));
        assert!(dot.is_instance_of::<PyValueError>(py));
        let empty = python_error(wire_failure(WireError::JobId(HttpError::Required {
            field_name: "job_id".to_owned(),
        })));
        assert!(empty.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn error_an_unknown_wire_failure_is_an_exception() {
    let failure = failure_of(wire_failure(WireError::Url {
        source: Url::parse("::").expect_err("the text has no scheme"),
    }));
    assert_eq!(failure.kind, "Exception");
}

#[test]
fn error_vertex_names_the_task() {
    let failure = failure_of(wire_failure(WireError::Vertex));
    assert_eq!(failure.kind, "Exception");
    assert!(failure.message.contains("B1384"));
}

#[test]
fn error_timeout_and_data_are_python_value_errors() {
    Python::initialize();
    Python::attach(|py| {
        let create_timeout = python_error(Raised::from(TuningFailure::Create(
            FineTuningCreateError::Timeout(TimeoutError::NonFinite),
        )));
        assert!(create_timeout.is_instance_of::<PyValueError>(py));
        let create_data = python_error(Raised::from(TuningFailure::Create(
            FineTuningCreateError::Data(json_error()),
        )));
        assert!(create_data.is_instance_of::<PyValueError>(py));
        let read_timeout = python_error(Raised::from(TuningFailure::Read(
            FineTuningReadError::Timeout(TimeoutError::NonFinite),
        )));
        assert!(read_timeout.is_instance_of::<PyValueError>(py));
        let cancel_timeout = python_error(Raised::from(TuningFailure::Cancel(
            FineTuningCancelError::Timeout(TimeoutError::NonFinite),
        )));
        assert!(cancel_timeout.is_instance_of::<PyValueError>(py));
    });
}

#[test]
fn error_read_failures_keep_their_class() {
    let failure = failure_of(Raised::from(TuningFailure::Read(
        FineTuningReadError::UnsupportedList {
            provider: "bedrock".to_owned(),
        },
    )));
    assert_eq!(failure.kind, "BadRequestError");
    assert!(failure.message.contains("'create_batch'"));
    let retrieve_failure = failure_of(Raised::from(TuningFailure::Read(
        FineTuningReadError::UnsupportedRetrieve {
            provider: "bedrock".to_owned(),
        },
    )));
    assert_eq!(retrieve_failure.kind, "BadRequestError");
    assert!(
        retrieve_failure
            .message
            .contains("'retrieve_fine_tuning_job'")
    );
}

#[test]
fn error_page_and_cancel_failures_keep_their_class() {
    let failure = failure_of(Raised::from(TuningFailure::Read(
        FineTuningReadError::Page {
            source: json_error(),
        },
    )));
    assert_eq!(failure.kind, "APIResponseValidationError");
    let cancel_failure = failure_of(Raised::from(TuningFailure::Cancel(
        FineTuningCancelError::Unsupported {
            provider: "bedrock".to_owned(),
        },
    )));
    assert_eq!(cancel_failure.kind, "BadRequestError");
    assert_eq!(cancel_failure.status_code, Some(400));
}

#[test]
fn error_a_dispatched_failure_keeps_the_native_class() {
    let failure = failure_of(Raised::from(TuningFailure::Create(
        FineTuningCreateError::Dispatch(WireError::MissingOpenAiKey),
    )));
    assert_eq!(failure.kind, "Exception");
    let read_failure = failure_of(Raised::from(TuningFailure::Read(
        FineTuningReadError::Dispatch(WireError::MissingAzureKey),
    )));
    assert_eq!(read_failure.kind, "Exception");
    assert!(read_failure.message.contains("AZURE_OPENAI_API_KEY"));
}

#[test]
fn error_a_read_call_needs_its_key() {
    let timeout = openai_timeout();
    let call = OpenAiReadCall {
        values: ReadValues {
            is_async: false,
            operation: ReadOperation::List {
                after: None,
                limit: None,
            },
            timeout: &timeout,
            max_retries: None,
        },
        api_base: "http://127.0.0.1:9/v1".to_owned(),
        api_key: None,
        api_version: None,
        organization: None,
    };
    let error =
        request_for(WireCall::OpenAiRead(&call), None).expect_err("no key means no request");
    assert!(matches!(error, WireError::MissingOpenAiKey));
}
