#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::vector_file_read::error::VectorFileReadError;
use betterlite_core::api::vector_file_write::error::{
    VectorFileWriteError, VectorFileWriteFailure, failure as write_failure,
};
use betterlite_core::api::vector_store_http_write::service::Operation as WriteOperation;
use betterlite_core::api::vector_store_search::error::SdkFailure;
use betterlite_error::envelope::WireField;
use betterlite_error::error::{ErrorClass, ErrorContext};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::list::list_request;
use super::read::read_call;
use super::write::{update_request, write_request};
use super::{
    CallError, CallFailure, ConvertError, CreateWire, ListWire, ReadWire, WriteWire,
    create_request, parse, payload_of,
};

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()
}

#[test]
fn request_create_carries_the_caller_fields() {
    let text = concat!(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","attributes":{"source":"doc.md"},"#,
        r#""chunking_strategy":{"type":"auto"},"extra_headers":{"X-Trace":"t"},"#,
        r#""extra_query":{"q":"1"},"extra_body":{"flag":true},"timeout":2.5,"#,
        r#""custom_llm_provider":"openai","kwargs":{"api_key":"sk-test"}}"#,
    );
    let wire: CreateWire = parse(text, 0).unwrap();
    let request = create_request(&wire);
    assert_eq!(request.transport.vector_store_id, "vs_a");
    assert_eq!(request.file_id, "file_1");
    assert_eq!(request.attributes.map(BTreeMap::len), Some(1));
    assert_eq!(request.chunking_strategy.map(IndexMap::len), Some(1));
    assert_eq!(
        request
            .transport
            .extra_headers
            .and_then(|headers| headers.get("X-Trace"))
            .map(String::as_str),
        Some("t")
    );
    assert_eq!(request.transport.extra_query.map(IndexMap::len), Some(1));
    assert_eq!(request.transport.extra_body.map(IndexMap::len), Some(1));
    assert_eq!(request.transport.timeout, Some(2.5_f64));
    assert_eq!(request.transport.custom_llm_provider, Some("openai"));
    assert_eq!(request.transport.kwargs.len(), 1);
}

#[test]
fn request_defaults_leave_the_optionals_absent() {
    let wire: CreateWire = parse(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","kwargs":{}}"#,
        0,
    )
    .unwrap();
    let request = create_request(&wire);
    assert!(request.attributes.is_none());
    assert!(request.chunking_strategy.is_none());
    assert!(request.transport.extra_headers.is_none());
    assert!(request.transport.extra_query.is_none());
    assert!(request.transport.extra_body.is_none());
    assert_eq!(request.transport.timeout, None);
    assert_eq!(request.transport.custom_llm_provider, None);
    assert!(request.transport.kwargs.is_empty());
}

#[test]
fn request_list_keeps_the_query_fields_and_sends_no_body() {
    let text = concat!(
        r#"{"vector_store_id":"vs_a","after":"file_1","before":"file_9","filter":"completed","#,
        r#""limit":3,"order":"desc","extra_headers":{"X-Trace":"t"},"extra_query":{"q":"1"},"#,
        r#""timeout":4.0,"custom_llm_provider":"openai","kwargs":{}}"#,
    );
    let wire: ListWire = parse(text, 0).unwrap();
    let request = list_request(&wire);
    assert_eq!(request.transport.vector_store_id, "vs_a");
    assert_eq!(request.after, Some("file_1"));
    assert_eq!(request.before, Some("file_9"));
    assert_eq!(request.filter, Some("completed"));
    assert_eq!(request.limit, Some(3));
    assert_eq!(request.order, Some("desc"));
    assert_eq!(request.transport.extra_query.map(IndexMap::len), Some(1));
    assert!(request.transport.extra_body.is_none());
    assert_eq!(request.transport.timeout, Some(4.0_f64));
}

#[test]
fn request_read_has_no_body_or_query() {
    let text = concat!(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","extra_headers":{"X-Trace":"t"},"#,
        r#""timeout":5.0,"custom_llm_provider":"openai","kwargs":{"api_key":"sk-test"}}"#,
    );
    let wire: ReadWire = parse(text, 0).unwrap();
    let call = read_call(&wire);
    assert_eq!(call.vector_store_id, "vs_a");
    assert_eq!(call.file_id, "file_1");
    assert_eq!(
        call.extra_headers
            .and_then(|headers| headers.get("X-Trace"))
            .map(String::as_str),
        Some("t")
    );
    assert_eq!(call.timeout, Some(5.0_f64));
    assert_eq!(call.custom_llm_provider, Some("openai"));
    assert_eq!(call.kwargs.len(), 1);
}

#[test]
fn request_update_adds_attributes_and_body_to_the_shared_write() {
    let text = concat!(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","attributes":{"source":"doc.md"},"#,
        r#""extra_headers":{"X-Trace":"t"},"extra_body":{"flag":true},"timeout":6.0,"#,
        r#""custom_llm_provider":"openai","kwargs":{}}"#,
    );
    let wire: WriteWire = parse(text, 0).unwrap();
    let request = update_request(&wire);
    assert_eq!(request.write.vector_store_id, "vs_a");
    assert_eq!(request.write.file_id, "file_1");
    assert_eq!(
        request
            .write
            .extra_headers
            .and_then(|headers| headers.get("X-Trace"))
            .map(String::as_str),
        Some("t")
    );
    assert_eq!(request.write.timeout, Some(6.0_f64));
    assert_eq!(request.attributes.map(BTreeMap::len), Some(1));
    assert_eq!(request.extra_body.map(IndexMap::len), Some(1));
}

#[test]
fn request_delete_shares_the_write_without_attributes() {
    let wire: WriteWire = parse(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","kwargs":{}}"#,
        0,
    )
    .unwrap();
    let request = write_request(&wire);
    assert_eq!(request.vector_store_id, "vs_a");
    assert_eq!(request.file_id, "file_1");
    assert!(request.extra_headers.is_none());
    assert_eq!(request.timeout, None);
    assert_eq!(request.custom_llm_provider, None);
    assert!(request.kwargs.is_empty());
    let updated = update_request(&wire);
    assert!(updated.attributes.is_none());
    assert!(updated.extra_body.is_none());
}

#[test]
fn request_update_sends_an_empty_attributes_map() {
    let wire: WriteWire = parse(
        r#"{"vector_store_id":"vs_a","file_id":"file_1","attributes":{},"kwargs":{}}"#,
        0,
    )
    .unwrap();
    let request = update_request(&wire);
    assert_eq!(request.attributes.map(BTreeMap::len), Some(0));
}

#[test]
fn error_attached_files_are_refused() {
    let result: Result<CreateWire, ConvertError> = parse("{}", 2);
    assert!(matches!(result, Err(ConvertError::Files { count: 2 })));
}

#[test]
fn error_unknown_fields_are_refused() {
    let text = r#"{"vector_store_id":"vs_a","file_id":"file_1","surprise":true}"#;
    let result: Result<CreateWire, ConvertError> = parse(text, 0);
    assert!(matches!(result, Err(ConvertError::Json { .. })));
}

#[test]
fn error_file_failure_becomes_the_payload() {
    let sdk = SdkFailure {
        class: String::from("AuthenticationError"),
        status: 401,
        kind: WireField::Text(String::from("invalid_request_error")),
        param: WireField::Null,
        code: WireField::Text(String::from("invalid_api_key")),
        message: String::from("bad key"),
    };
    let error = CallError {
        provider: String::from("openai"),
        source: CallFailure::File(Box::new(sdk)),
    };
    let payload = payload_of(&error, &redactor());
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.message, "bad key");
    assert_eq!(payload.status_code, 401);
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model, "");
}

#[test]
fn error_read_failure_is_projected() {
    let error = CallError {
        provider: String::from("azure"),
        source: CallFailure::Read(VectorFileReadError::Unsupported {
            operation: "retrieve",
            provider: String::from("azure"),
        }),
    };
    let payload = payload_of(&error, &redactor());
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.status_code, 500);
    assert_eq!(
        payload.message,
        "Vector store file retrieve is not supported for azure"
    );
    assert_eq!(payload.llm_provider, "azure");
}

#[test]
fn error_write_failure_is_projected() {
    let source = write_failure(
        VectorFileWriteError::Unsupported {
            operation: WriteOperation::Update,
            provider: String::from("azure"),
        },
        Some("azure"),
        &redactor(),
    );
    let error = CallError {
        provider: String::from("azure"),
        source: CallFailure::Write(Box::new(source)),
    };
    let payload = payload_of(&error, &redactor());
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.status_code, 500);
    assert!(
        payload
            .message
            .contains("Vector store file update is not supported for azure")
    );
    assert_eq!(payload.llm_provider, "azure");
}

#[test]
fn error_unprojected_write_class_falls_back_to_api_connection() {
    let class = ErrorClass::Unclassified(Box::new(ErrorContext {
        message: String::from("attributes must be a mapping"),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let source = VectorFileWriteFailure {
        error: VectorFileWriteError::FieldType {
            field: "attributes",
            expected: "a mapping",
        },
        class,
    };
    let error = CallError {
        provider: String::from("openai"),
        source: CallFailure::Write(Box::new(source)),
    };
    let payload = payload_of(&error, &redactor());
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.status_code, 500);
    assert_eq!(payload.message, "attributes must be a mapping");
}
