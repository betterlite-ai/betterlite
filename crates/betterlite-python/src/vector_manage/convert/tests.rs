#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap as WireHeaders, WriteError,
};
use betterlite_core::api::vector_store_read::error::{
    VectorStoreReadError, failure as read_failure,
};
use betterlite_core::api::vector_store_read::service::Operation as ReadOperation;
use betterlite_core::api::vector_store_write::error::{
    VectorStoreWriteError, failure as write_failure,
};
use betterlite_core::api::vector_store_write::service::Operation as WriteOperation;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::requests::{delete_request, list_request, retrieve_request, update_request};
use super::{
    ConvertError, DeleteWire, ListWire, NativeErrorPayload, RetrieveWire, UpdateWire, params,
    parse, read_payload, write_payload,
};

const RETRIEVE_TEXT: &str = r#"{"vector_store_id":"vs_1","kwargs":{"api_key":"sk-test","api_base":"http://127.0.0.1:9/v1","ssl_verify":true},"extra_headers":{"x-test":"1"},"extra_query":{"q":"x"},"extra_body":{"custom":true},"timeout":12.0,"custom_llm_provider":"openai"}"#;
const LIST_TEXT: &str = r#"{"after":"a","before":null,"limit":null,"order":"asc","kwargs":{"api_key":"sk-test"},"timeout":null}"#;
const UPDATE_TEXT: &str = r#"{"vector_store_id":"vs_1","name":"notes","expires_after":{"anchor":"last_active_at","days":7},"metadata":{},"kwargs":{"file_ids":["file-1"]}}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

fn exception(status_code: u16) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: String::from("missing"),
        headers: WireHeaders::new(),
        status_code_is_synthesized: false,
    }
}

#[test]
fn request_retrieve_keeps_every_field() {
    let wire: RetrieveWire = parse(RETRIEVE_TEXT, 0).expect("the retrieve wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = retrieve_request(&wire, &write);
    assert_eq!(request.vector_store_id, "vs_1");
    assert_eq!(
        request.transport.params.api_base.as_deref(),
        Some("http://127.0.0.1:9/v1")
    );
    assert_eq!(request.transport.params.ssl_verify, Some(true));
    assert!(request.transport.params.api_key.is_some());
    assert_eq!(
        request.transport.extra_headers,
        Some(&BTreeMap::from([(
            String::from("x-test"),
            String::from("1")
        )]))
    );
    assert_eq!(
        request.transport.extra_query,
        Some(&IndexMap::from([(String::from("q"), json(r#""x""#))]))
    );
    assert_eq!(
        request.transport.extra_body,
        Some(&IndexMap::from([(String::from("custom"), json("true"))]))
    );
    assert_eq!(request.transport.timeout, Some(12.0_f64));
    assert_eq!(request.transport.custom_llm_provider, Some("openai"));
    assert_eq!(request.transport.kwargs.len(), 3);
}

#[test]
fn request_retrieve_defaults_the_absent_fields() {
    let wire: RetrieveWire =
        parse(r#"{"vector_store_id":"vs"}"#, 0).expect("a bare retrieve wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = retrieve_request(&wire, &write);
    assert_eq!(request.vector_store_id, "vs");
    assert_eq!(request.transport.extra_headers, None);
    assert_eq!(request.transport.extra_query, None);
    assert_eq!(request.transport.extra_body, None);
    assert_eq!(request.transport.timeout, None);
    assert_eq!(request.transport.custom_llm_provider, None);
    assert_eq!(request.transport.params.api_base, None);
    assert_eq!(request.transport.params.api_key, None);
    assert_eq!(request.transport.params.ssl_verify, None);
}

#[test]
fn request_list_keeps_a_null_distinct_from_an_absent_field() {
    let wire: ListWire = parse(LIST_TEXT, 0).expect("the list wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = list_request(&wire, &write);
    assert_eq!(request.after, Some("a"));
    assert_eq!(request.before, None);
    assert_eq!(request.limit, Some(None));
    assert_eq!(request.order, Some(Some("asc")));
}

#[test]
fn request_list_defaults_the_absent_fields() {
    let wire: ListWire = parse("{}", 0).expect("an empty list wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = list_request(&wire, &write);
    assert_eq!(request.after, None);
    assert_eq!(request.before, None);
    assert_eq!(request.limit, None);
    assert_eq!(request.order, None);
}

#[test]
fn request_list_carries_its_four_values() {
    let wire: ListWire = parse(r#"{"after":"a","before":"b","limit":5,"order":"asc"}"#, 0)
        .expect("the list wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = list_request(&wire, &write);
    assert_eq!(request.after, Some("a"));
    assert_eq!(request.before, Some("b"));
    assert_eq!(request.limit, Some(Some(5)));
    assert_eq!(request.order, Some(Some("asc")));
}

#[test]
fn request_update_keeps_an_empty_metadata() {
    let wire: UpdateWire = parse(UPDATE_TEXT, 0).expect("the update wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = update_request(&wire, &write);
    assert_eq!(request.transport.vector_store_id, "vs_1");
    assert_eq!(request.name, Some("notes"));
    let expires = serde_json::to_string(request.expires_after.expect("the policy parses"))
        .expect("the policy serializes");
    assert_eq!(expires, r#"{"anchor":"last_active_at","days":7}"#);
    assert_eq!(
        request.metadata.as_ref().map(|metadata| metadata.len()),
        Some(0)
    );
    assert_eq!(
        request.transport.kwargs.get("file_ids"),
        Some(&json(r#"["file-1"]"#))
    );
}

#[test]
fn request_update_leaves_a_null_field_out() {
    let wire: UpdateWire = parse(
        r#"{"vector_store_id":"vs","name":null,"expires_after":null,"metadata":null}"#,
        0,
    )
    .expect("the update wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = update_request(&wire, &write);
    assert_eq!(request.name, None);
    assert!(request.expires_after.is_none());
    assert!(request.metadata.is_none());
}

#[test]
fn request_delete_carries_the_id_alone() {
    let wire: DeleteWire = parse(
        r#"{"vector_store_id":"vs_1","kwargs":{"api_key":"sk-test"}}"#,
        0,
    )
    .expect("the delete wire parses");
    let write = params(&wire.kwargs).expect("the transport parameters read");
    let request = delete_request(&wire, &write);
    assert_eq!(request.transport.vector_store_id, "vs_1");
    assert_eq!(request.name, None);
    assert!(request.expires_after.is_none());
    assert!(request.metadata.is_none());
    assert_eq!(request.transport.kwargs.len(), 1);
}

#[test]
fn request_wire_rejects_an_unknown_key() {
    let refused = parse::<RetrieveWire>(r#"{"vector_store_id":"vs","router":null}"#, 0)
        .expect_err("a key the call does not carry is refused");
    assert!(matches!(refused, ConvertError::Json { .. }));
}

#[test]
fn request_wire_rejects_attached_files() {
    let refused = parse::<DeleteWire>(r#"{"vector_store_id":"vs"}"#, 1)
        .expect_err("a vector store call takes no file");
    assert!(matches!(refused, ConvertError::Files { count: 1 }));
}

#[test]
fn request_wire_requires_its_id() {
    assert!(matches!(
        parse::<RetrieveWire>("{}", 0),
        Err(ConvertError::Json { .. })
    ));
    assert!(matches!(
        parse::<ListWire>("not json", 0),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn request_params_reads_the_three_transport_fields() {
    let kwargs = IndexMap::from([
        (String::from("api_base"), json(r#""http://127.0.0.1:9/v1""#)),
        (String::from("api_key"), json(r#""sk-test""#)),
        (String::from("ssl_verify"), json("false")),
        (String::from("litellm_call_id"), json(r#""call-1""#)),
    ]);
    let write = params(&kwargs).expect("the transport parameters read");
    assert_eq!(write.api_base.as_deref(), Some("http://127.0.0.1:9/v1"));
    assert_eq!(write.ssl_verify, Some(false));
    assert!(write.api_key.is_some());
}

#[test]
fn error_a_wrong_api_base_names_the_field() {
    let kwargs = IndexMap::from([(String::from("api_base"), json("5"))]);
    let refused = params(&kwargs).expect_err("a number is not an api_base");
    assert_eq!(refused.to_string(), "api_base must be a string");
}

#[test]
fn error_a_wrong_ssl_verify_names_the_field() {
    let kwargs = IndexMap::from([(String::from("ssl_verify"), json(r#""yes""#))]);
    let refused = params(&kwargs).expect_err("text is not an ssl_verify");
    assert_eq!(refused.to_string(), "ssl_verify must be a boolean");
}

#[test]
fn error_a_wrong_api_key_names_the_field() {
    let kwargs = IndexMap::from([(String::from("api_key"), json("[]"))]);
    let refused = params(&kwargs).expect_err("a list is not an api_key");
    assert_eq!(refused.to_string(), "api_key must be a string");
}

#[test]
fn error_read_failure_is_a_status_class() {
    let error =
        VectorStoreReadError::Read(CreateReadError::Write(WriteError::Provider(exception(401))));
    let failure = read_failure(error, "openai", &redactor());
    assert_eq!(
        read_payload(&failure, "openai"),
        NativeErrorPayload {
            kind: String::from("AuthenticationError"),
            message: String::from("AuthenticationError: OpenAIException - missing"),
            status_code: 401,
            llm_provider: String::from("openai"),
            model: String::new(),
        }
    );
}

#[test]
fn error_unconfigured_read_is_an_api_connection_error() {
    let error = VectorStoreReadError::Unconfigured {
        operation: ReadOperation::Retrieve,
        provider: String::from("anthropic"),
    };
    let failure = read_failure(error, "anthropic", &redactor());
    let payload = read_payload(&failure, "anthropic");
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.llm_provider, "anthropic");
    assert_eq!(payload.status_code, 500);
    assert_eq!(
        payload.message,
        "AnthropicException - Vector store retrieve is not supported for anthropic"
    );
}

#[test]
fn error_write_failure_is_a_status_class() {
    let error = VectorStoreWriteError::Write(WriteError::Provider(exception(404)));
    let failure = write_failure(error, "openai", &redactor());
    let payload = write_payload(&failure, "openai");
    assert_eq!(payload.kind, "NotFoundError");
    assert_eq!(payload.message, "NotFoundError: OpenAIException - missing");
    assert_eq!(payload.status_code, 404);
    assert_eq!(payload.llm_provider, "openai");
    assert_eq!(payload.model, String::new());
}

#[test]
fn error_unsupported_write_is_an_api_connection_error() {
    let error = VectorStoreWriteError::Unsupported {
        operation: WriteOperation::Delete,
        provider: String::from("anthropic"),
    };
    let failure = write_failure(error, "anthropic", &redactor());
    let payload = write_payload(&failure, "anthropic");
    assert_eq!(payload.kind, "APIConnectionError");
    assert_eq!(payload.llm_provider, "anthropic");
    assert_eq!(
        payload.message,
        "AnthropicException - Vector store delete is not supported for anthropic"
    );
}
