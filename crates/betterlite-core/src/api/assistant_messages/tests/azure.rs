use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;

use super::super::error::{MessageError, SetupError};
use super::super::options::OptionError;
use super::super::service::{MessageRequest, Operation};
use super::{Fixture, MESSAGE_REPLY, call, data, json, params, request};

const AZURE_URL: &str =
    "https://azure.test/openai/threads/thread_a/messages?api-version=2024-05-01-preview";

#[test]
fn azure_attachment_drop_and_content() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(
        r#"{"role":"user","content":"hello","attachments":[{"file_id":"file_a"}],"metadata":{"tag":"x"}}"#,
    );
    call(
        &fixture,
        &request("azure", Some(&message), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap();
    let seen = fixture.transport.requests();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, AZURE_URL);
    assert_eq!(
        seen[0].headers.get("api-key").map(String::as_str),
        Some("sk-az")
    );
    assert_eq!(
        seen[0].body,
        Some(json(
            r#"{"role":"user","content":"hello","metadata":{"tag":"x"}}"#
        ))
    );
}

#[test]
fn azure_attachment_drop_and_content_rejects_unsupported_attachments() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let error = call(
        &fixture,
        &request("azure", Some(&message), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Azure doesn't support [{'tools': []}]. To drop it from the call, set `litellm.drop_params = True."
    );
    let MessageError::Setup(SetupError::Options(options)) = error.error else {
        panic!("expected the selector error");
    };
    assert_eq!(
        options.litellm_message(),
        format!("litellm.UnsupportedParamsError: {options}")
    );
    assert_eq!(options.status(), Some(400));
    assert_eq!(options.exception_name(), "UnsupportedParamsError");
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn azure_attachment_drop_and_content_drops_invalid_attachments_with_drop_params() {
    let fixture = Fixture::new();
    fixture.transport.queue(200, MESSAGE_REPLY);
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    call(
        &fixture,
        &MessageRequest {
            provider: "azure",
            thread_id: "thread_a",
            data: Some(&message),
            params: &params("azure"),
            drop_params: true,
        },
        Operation::Append,
        false,
    )
    .unwrap();
    assert_eq!(
        fixture.transport.requests()[0].body,
        Some(json(r#"{"role":"user","content":"hello"}"#))
    );
}

#[test]
fn azure_attachment_drop_and_content_rejects_non_list_attachments() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":{}}"#);
    let error = call(
        &fixture,
        &request("azure", Some(&message), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid param. attachments should always be a list. Got=<class 'dict'>, Expected=List. Raw value={}"
    );
    assert!(matches!(
        error.error,
        MessageError::Setup(SetupError::Options(OptionError::AttachmentsNotList { .. }))
    ));
    let float = data(r#"{"role":"user","content":"hello","attachments":5.5}"#);
    let error = call(
        &fixture,
        &request("azure", Some(&float), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid param. attachments should always be a list. Got=<class 'float'>, Expected=List. Raw value=5.5"
    );
    let negative = data(r#"{"role":"user","content":"hello","attachments":-5}"#);
    let error = call(
        &fixture,
        &request("azure", Some(&negative), &params("azure")),
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Invalid param. attachments should always be a list. Got=<class 'int'>, Expected=List. Raw value=-5"
    );
}

#[test]
fn azure_attachment_drop_and_content_rejects_content_lists_even_with_drop_params() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":[{"type":"text","text":"hello"}]}"#);
    let error = call(
        &fixture,
        &MessageRequest {
            provider: "azure",
            thread_id: "thread_a",
            data: Some(&message),
            params: &params("azure"),
            drop_params: true,
        },
        Operation::Append,
        false,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Azure only accepts content as a string.");
    assert!(matches!(
        error.error,
        MessageError::Setup(SetupError::Options(OptionError::ContentNotString))
    ));
    assert!(fixture.transport.requests().is_empty());
}

#[test]
fn azure_attachment_drop_and_content_maps_async_failures() {
    let fixture = Fixture::new();
    let message = data(r#"{"role":"user","content":"hello","attachments":[{"tools":[]}]}"#);
    let failure = call(
        &fixture,
        &request("azure", Some(&message), &params("azure")),
        Operation::Append,
        true,
    )
    .unwrap_err();
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.error_type, WireField::MISSING);
    assert_eq!(metadata.param, WireField::Null);
    assert_eq!(metadata.openai_code, None);
    assert_eq!(
        failure.to_string(),
        "Azure doesn't support [{'tools': []}]. To drop it from the call, set `litellm.drop_params = True."
    );
    assert!(matches!(failure.class, ErrorClass::UnsupportedParams(_)));
}
