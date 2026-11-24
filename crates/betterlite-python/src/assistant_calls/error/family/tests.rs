#![cfg(test)]

use betterlite_core::api::assistant_read_delete::error::AssistantReadDeleteError;
use betterlite_core::api::assistant_read_delete::sdk_dispatch::AssistantSdkError;
use betterlite_core::api::assistant_records::error::{AssistantCreateFailure, unsupported};
use betterlite_core::api::assistant_threads::error::{ProviderFailure, ThreadError};
use betterlite_error::envelope::WireField;
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::threads::error::ThreadsError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::{ThreadOutcome, create, read_delete, thread};

#[test]
fn error_payload_for_an_unsupported_create_names_its_class_and_model() {
    let failure = unsupported("bedrock");
    let text = serde_json::to_string(&create(&failure, "gpt-4o-mini", "bedrock")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"BadRequestError","message":"LiteLLM doesn't support bedrock for 'create_assistants'. Only 'openai' is supported.","status_code":400,"llm_provider":"bedrock","model":"n/a","body":{"type":null,"param":null,"code":null}}"#
    );
}

#[test]
fn error_payload_for_a_plain_create_failure_keeps_the_request_model() {
    let failure = AssistantCreateFailure {
        class: String::from("BadRequestError"),
        status: Some(400),
        kind: WireField::Text(String::from("invalid_request_error")),
        param: WireField::MISSING,
        code: WireField::Null,
        message: String::from("the model is unavailable"),
    };
    let text = serde_json::to_string(&create(&failure, "gpt-4o-mini", "openai")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"BadRequestError","message":"the model is unavailable","status_code":400,"llm_provider":"openai","model":"gpt-4o-mini","body":{"type":"invalid_request_error","code":null}}"#
    );
}

#[test]
fn error_payload_maps_every_raw_sdk_class_python_cannot_build() {
    let cases = [
        ("APITimeoutError", "Timeout"),
        ("ConflictError", "APIError"),
        ("APIStatusError", "APIError"),
        ("OpenAIError", "AuthenticationError"),
        ("ValueError", "APIConnectionError"),
        ("BadRequestError", "BadRequestError"),
    ];
    for (class, expected) in cases {
        let failure = AssistantCreateFailure {
            class: String::from(class),
            status: None,
            kind: WireField::MISSING,
            param: WireField::MISSING,
            code: WireField::MISSING,
            message: String::from("the call failed"),
        };
        let text = serde_json::to_string(&create(&failure, "gpt-4o-mini", "openai")).unwrap();
        let expected_text = format!(
            r#"{{"type":"{expected}","message":"the call failed","llm_provider":"openai","model":"gpt-4o-mini"}}"#
        );
        assert_eq!(text, expected_text);
    }
}

#[test]
fn error_payload_for_a_missing_key_reads_as_an_authentication_error() {
    let error: AssistantReadDeleteError<AssistantSdkError> = AssistantReadDeleteError::Dispatch(
        AssistantSdkError::Assistants(AssistantsError::MissingApiKey),
    );
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = serde_json::to_string(&read_delete(error, "openai", &redactor)).unwrap();
    assert_eq!(
        text,
        r#"{"type":"AuthenticationError","message":"The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable","llm_provider":"openai","model":""}"#
    );
}

#[test]
fn error_payload_for_a_sync_thread_timeout_maps_the_sdk_class() {
    let error = ThreadError::ProviderTimeout(ThreadsError::MissingApiKey);
    let text = serde_json::to_string(&thread(&ThreadOutcome::Sync(error), "openai")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"Timeout","message":"the thread call failed: the api_key client option must be set","status_code":408,"llm_provider":"openai"}"#
    );
}

#[test]
fn error_payload_for_a_thread_conflict_maps_the_sdk_class() {
    let error = ThreadError::Provider {
        message: String::from("conflict"),
        failure: Box::new(ProviderFailure {
            status: 409,
            body_text: String::from("{}"),
            kind: None,
            param: None,
            code: None,
            body_message: None,
        }),
    };
    let text = serde_json::to_string(&thread(&ThreadOutcome::Sync(error), "openai")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"APIError","message":"conflict","status_code":409,"llm_provider":"openai"}"#
    );
}

#[test]
fn error_payload_for_a_mismatched_reply_is_an_internal_error() {
    let error: AssistantReadDeleteError<AssistantSdkError> =
        AssistantReadDeleteError::MismatchedReply;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let text = serde_json::to_string(&read_delete(error, "openai", &redactor)).unwrap();
    assert_eq!(
        text,
        r#"{"type":"InternalServerError","message":"the dispatch answered with the wrong reply","status_code":500,"llm_provider":"openai","model":"n/a","body":{"type":"internal_server_error"}}"#
    );
}

#[test]
fn error_payload_for_an_unsupported_thread_names_its_class_and_model() {
    let error = ThreadError::Unsupported {
        provider: String::from("bedrock"),
        operation: "create_thread",
    };
    let text = serde_json::to_string(&thread(&ThreadOutcome::Sync(error), "bedrock")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"BadRequestError","message":"LiteLLM doesn't support bedrock for 'create_thread'. Only 'openai' is supported.","status_code":400,"llm_provider":"bedrock","model":"n/a"}"#
    );
}

#[test]
fn error_payload_for_a_rate_limited_thread_keeps_the_body_type() {
    let error = ThreadError::Provider {
        message: String::from("slow down"),
        failure: Box::new(ProviderFailure {
            status: 429,
            body_text: String::from("{}"),
            kind: Some(String::from("rate_limit_error")),
            param: None,
            code: None,
            body_message: None,
        }),
    };
    let text = serde_json::to_string(&thread(&ThreadOutcome::Sync(error), "openai")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"RateLimitError","message":"slow down","status_code":429,"llm_provider":"openai","body":{"type":"rate_limit_error"}}"#
    );
}

#[test]
fn error_payload_for_a_plain_bad_request_thread_leaves_the_model_unset() {
    let error = ThreadError::Provider {
        message: String::from("boom"),
        failure: Box::new(ProviderFailure {
            status: 400,
            body_text: String::from("{}"),
            kind: None,
            param: None,
            code: None,
            body_message: None,
        }),
    };
    let text = serde_json::to_string(&thread(&ThreadOutcome::Sync(error), "openai")).unwrap();
    assert_eq!(
        text,
        r#"{"type":"BadRequestError","message":"boom","status_code":400,"llm_provider":"openai"}"#
    );
}
