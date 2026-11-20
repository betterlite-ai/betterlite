use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_openai::threads::error::ThreadsError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::api::resource_sdk_settings::ResourceSdkSettings;

use super::super::connection::{MessageParams, PrepareInput, Route, prepare};
use super::super::error::{MessageError, SetupError};
use super::super::options::{MessageData, OptionError};
use super::super::service::{MessageRequest, Operation};
use super::{Fixture, MESSAGE_REPLY, call, data, empty_settings, json, params, request};

const NOT_FOUND_BODY: &str = r#"{"error":{"message":"no thread","type":"invalid_request_error","param":"thread_id","code":"model_not_found"}}"#;
const BAD_REQUEST_BODY: &str = r#"{"error":{"message":"bad role","type":"invalid_request_error","param":"role","code":"unsupported_value"}}"#;

#[test]
fn message_settings_and_failures_resolves_openai_credentials() {
    let mut fixture = Fixture::new();
    fixture.settings.api_key = Some(SecretString::from("sk-global"));
    fixture.settings.organization = Some(String::from("global-org"));
    fixture.settings.openai_organization = Some(String::from("env-org"));
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    call(
        &fixture,
        &request("openai", Some(&message), &MessageParams::default()),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-global")
    );
    assert_eq!(
        seen[0]
            .headers
            .get("openai-organization")
            .map(String::as_str),
        Some("global-org")
    );
}

#[test]
fn message_settings_and_failures_prefers_request_credentials() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut request_params = params("openai");
    request_params.api_key = Some(SecretString::from("sk-request"));
    request_params.organization = Some(String::from("request-org"));
    call(
        &fixture,
        &request("openai", Some(&message), &request_params),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer sk-request")
    );
    assert_eq!(
        seen[0]
            .headers
            .get("openai-organization")
            .map(String::as_str),
        Some("request-org")
    );
}

#[test]
fn message_settings_and_failures_defaults_the_deadline() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap();
    assert_eq!(
        fixture.transport.requests()[0].deadline,
        Duration::from_secs(600)
    );
}

#[test]
fn message_settings_and_failures_reads_timeouts() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut zero = params("openai");
    zero.timeout = Some(serde_json::from_str("0").unwrap());
    zero.request_timeout = Some(30.0_f64);
    call(
        &fixture,
        &request("openai", Some(&message), &zero),
        Operation::Append,
        false,
    )
    .unwrap();
    let mut text = params("openai");
    text.timeout = Some(serde_json::from_str(r#""12.5""#).unwrap());
    call(
        &fixture,
        &request("openai", Some(&message), &text),
        Operation::Append,
        false,
    )
    .unwrap();
    let mut structured = params("openai");
    structured.timeout = Some(serde_json::from_str(r#"{"read":5,"connect":1}"#).unwrap());
    call(
        &fixture,
        &request("openai", Some(&message), &structured),
        Operation::Append,
        false,
    )
    .unwrap();
    let mut empty = params("openai");
    empty.timeout = Some(serde_json::from_str(r#""""#).unwrap());
    empty.request_timeout = Some(45.0_f64);
    call(
        &fixture,
        &request("openai", Some(&message), &empty),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(seen[0].deadline, Duration::from_secs(30));
    assert_eq!(seen[1].deadline, Duration::from_millis(12_500));
    assert_eq!(seen[2].deadline, Duration::from_secs(5));
    assert_eq!(seen[3].deadline, Duration::from_secs(45));
}

#[test]
fn message_settings_and_failures_reads_the_azure_timeout() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut scalar = params("azure");
    scalar.timeout = Some(serde_json::from_str("12.5").unwrap());
    call(
        &fixture,
        &request("azure", Some(&message), &scalar),
        Operation::Append,
        false,
    )
    .unwrap();
    let mut text = params("azure");
    text.timeout = Some(serde_json::from_str(r#""8""#).unwrap());
    call(
        &fixture,
        &request("azure", Some(&message), &text),
        Operation::Append,
        false,
    )
    .unwrap();
    let mut structured = params("azure");
    structured.timeout = Some(serde_json::from_str(r#"{"read":5,"connect":1}"#).unwrap());
    call(
        &fixture,
        &request("azure", Some(&message), &structured),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(seen[0].deadline, Duration::from_millis(12_500));
    assert_eq!(seen[1].deadline, Duration::from_secs(8));
    assert_eq!(seen[2].deadline, Duration::from_secs(5));
}

fn facade_token(
    message: &MessageData,
    params: &MessageParams,
    settings: &ResourceSdkSettings,
) -> Option<String> {
    let input = PrepareInput {
        provider: "azure",
        operation: "create_thread",
        data: Some(message),
        params,
        settings,
        drop_params: false,
        asynchronous: false,
    };
    let prepared = prepare(&input).unwrap();
    let Route::Azure(credentials) = prepared.route else {
        panic!("expected the azure route");
    };
    credentials
        .facade_ad_token
        .map(|token| token.expose_secret().to_owned())
}

#[test]
fn message_settings_and_failures_selects_the_azure_ad_token() {
    let mut settings = empty_settings();
    settings.azure_ad_token = Some(SecretString::from("env-token"));
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut params = params("azure");
    assert_eq!(facade_token(&message, &params, &settings), None);
    params.extra_body = Some(Some(BTreeMap::new()));
    assert_eq!(facade_token(&message, &params, &settings), None);
    params.extra_body = Some(None);
    assert_eq!(
        facade_token(&message, &params, &settings),
        Some(String::from("env-token"))
    );
    params.extra_body = Some(Some(BTreeMap::from([(
        String::from("azure_ad_token"),
        json(r#""body-token""#),
    )])));
    assert_eq!(
        facade_token(&message, &params, &settings),
        Some(String::from("body-token"))
    );
}

#[test]
fn message_settings_and_failures_sends_the_azure_ad_token() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let mut params = params("azure");
    params.azure_ad_token = Some(SecretString::from("ad-token"));
    call(
        &fixture,
        &request("azure", Some(&message), &params),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(
        seen[0].headers.get("authorization").map(String::as_str),
        Some("Bearer ad-token")
    );
    assert!(!seen[0].headers.contains_key("api-key"));
}

#[test]
fn message_settings_and_failures_names_the_append_operation() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let error = call(
        &fixture,
        &request("anthropic", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'create_thread'. Only 'openai' is supported."
    );
    let failure = call(
        &fixture,
        &request("anthropic", Some(&message), &params("openai")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.model.as_deref(), Some("n/a"));
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

#[test]
fn message_settings_and_failures_names_the_list_operation() {
    let fixture = Fixture::new();
    let error = call(
        &fixture,
        &request("anthropic", None, &params("openai")),
        Operation::List,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "LiteLLM doesn't support anthropic for 'get_messages'. Only 'openai' is supported."
    );
    let failure = call(
        &fixture,
        &request("anthropic", None, &params("openai")),
        Operation::List,
        true,
    )
    .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "LiteLLM doesn't support anthropic for 'get_messages'. Only 'openai' is supported."
    );
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
}

#[test]
fn message_settings_and_failures_preserves_the_provider_body() {
    let fixture = Fixture::new();
    fixture.transport.queue(404, NOT_FOUND_BODY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let error = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.error.provider_answer(),
        Some((404_u16, NOT_FOUND_BODY.as_bytes()))
    );
    assert!(matches!(
        error.error,
        MessageError::OpenAi(ThreadsError::Status { status: 404, .. })
    ));
}

#[test]
fn message_settings_and_failures_rejects_an_unusable_thread_id() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let empty = call(
        &fixture,
        &MessageRequest {
            provider: "openai",
            thread_id: "",
            data: Some(&message),
            params: &params("openai"),
            drop_params: false,
        },
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        empty.error,
        MessageError::OpenAi(ThreadsError::EmptyThreadId)
    ));
    let dotted = call(
        &fixture,
        &MessageRequest {
            provider: "openai",
            thread_id: "..",
            data: Some(&message),
            params: &params("openai"),
            drop_params: false,
        },
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        dotted.error,
        MessageError::OpenAi(ThreadsError::DotSegment { .. })
    ));
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn message_settings_and_failures_maps_the_async_not_found() {
    let fixture = Fixture::new();
    fixture.transport.queue(404, NOT_FOUND_BODY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let failure = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.class, ErrorClass::NotFound(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 404);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

// the SDK unwraps body["error"] before APIError reads type/param/code, and LiteLLM forwards that unwrapped body
#[test]
fn message_settings_and_failures_keeps_the_async_body_fields() {
    let fixture = Fixture::new();
    fixture.transport.queue(400, BAD_REQUEST_BODY);
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let failure = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(
        metadata.error_type,
        WireField::Text(String::from("invalid_request_error"))
    );
    assert_eq!(metadata.param, WireField::Text(String::from("role")));
    assert_eq!(metadata.openai_code.as_deref(), Some("unsupported_value"));
}

// APIError.__init__ reads no fields off a body that is not a mapping, so the whole object becomes the body
#[test]
fn message_settings_and_failures_ignores_a_non_object_error() {
    let fixture = Fixture::new();
    fixture.transport.queue(
        400,
        r#"{"error":"boom","type":"invalid_request_error","param":"role","code":"stale"}"#,
    );
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let failure = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::Null);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
}

// get_error_message falls back to the message of a body["error"] object nested inside the unwrapped body
#[test]
fn message_settings_and_failures_reads_a_nested_error_message() {
    let fixture = Fixture::new();
    fixture.transport.queue(
        400,
        r#"{"error":{"error":{"message":"deep failure"},"type":"invalid_request_error"}}"#,
    );
    let message = data(r#"{"role":"user","content":"hello"}"#);
    let failure = call(
        &fixture,
        &request("openai", Some(&message), &params("openai")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    let ErrorClass::BadRequest(details) = failure.class else {
        panic!("the 400 must map to the bad request class");
    };
    assert_eq!(details.message, "OpenAIException - deep failure");
}

#[test]
fn message_settings_and_failures_runs_the_selector_before_the_provider() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let error = call(
        &fixture,
        &request("azure", Some(&message), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert!(matches!(
        error.error,
        MessageError::Setup(SetupError::Options(
            OptionError::UnsupportedAttachment { .. }
        ))
    ));
    assert!(fixture.transport.requests().is_empty());
}
