#![cfg(test)]

use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{Arguments, ConvertError, parse, query_values};

const LIST: &str = r#"{
    "container_id": "cntr_1",
    "after": "cntr_0",
    "limit": 3,
    "order": "asc",
    "timeout": 5.5,
    "api_key": "sk-test",
    "api_base": "http://localhost:1/v1",
    "api_version": "2024-01-01",
    "custom_llm_provider": "openai",
    "extra_headers": {"X-Probe": "yes"},
    "extra_query": {"page": "2", "limit": 4},
    "extra_body": {"target_model_names": "gpt-4o"},
    "litellm_call_id": "call_1",
    "batch_size": 7
}"#;

const UPLOAD: &str = r#"{
    "container_id": "cntr_1",
    "file": {"$file": {"name": "data.csv", "blob": 0, "content_type": "text/csv"}},
    "timeout": 600
}"#;

fn arguments(text: &str) -> Arguments {
    match parse::<Arguments>(text) {
        Ok(arguments) => arguments,
        Err(error) => panic!("the request should parse: {error}"),
    }
}

#[test]
fn request_list_keeps_the_named_arguments_out_of_kwargs() {
    let call = arguments(LIST);
    assert_eq!(call.container_id.as_deref(), Some("cntr_1"));
    assert_eq!(call.after.as_deref(), Some("cntr_0"));
    assert_eq!(call.limit, Some(3));
    assert_eq!(call.order.as_deref(), Some("asc"));
    assert_eq!(call.timeout, Some(5.5_f64));
    assert_eq!(call.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(call.extra.len(), 2);
    assert_eq!(
        call.extra.get("batch_size"),
        Some(&Json::Number(Number::from(7_i32)))
    );
    assert!(call.extra.contains_key("litellm_call_id"));
    assert!(!call.extra.contains_key("extra_body"));
    let converted = match call.endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(converted.container_id.as_deref(), Some("cntr_1"));
    assert_eq!(converted.limit, Some(3));
    assert_eq!(converted.extra.len(), 2);
}

#[test]
fn request_query_values_carry_text_and_integers() {
    let call = arguments(LIST);
    let query = match query_values(call.extra_query.as_ref()) {
        Ok(Some(query)) => query,
        Ok(None) => panic!("the request names a query"),
        Err(error) => panic!("the query should convert: {error}"),
    };
    assert_eq!(query.get("page"), Some(&ParamValue::Text("2".to_owned())));
    assert_eq!(query.get("limit"), Some(&ParamValue::Integer(4)));
}

#[test]
fn request_upload_moves_the_blob_into_a_tuple_file() {
    let call = arguments(UPLOAD);
    let mut blobs = vec![b"a,b\n1,2\n".to_vec()];
    let file = match call.upload(&mut blobs) {
        Ok(file) => file,
        Err(error) => panic!("the file should convert: {error}"),
    };
    let expected = FileData::Tuple3 {
        filename: Some("data.csv".to_owned()),
        content: FileBody::Bytes(b"a,b\n1,2\n".to_vec()),
        content_type: Some("text/csv".to_owned()),
    };
    assert_eq!(file.file_data, expected);
    assert!(blobs[0].is_empty());
    assert!(call.extra.is_empty());
}

#[test]
fn request_endpoint_call_defaults_the_provider_and_reads_the_call_id() {
    let call = match parse::<Arguments>(r#"{"file_id": "file_1", "litellm_call_id": "call_1"}"#) {
        Ok(call) => call,
        Err(error) => panic!("the request should parse: {error}"),
    };
    let converted = match call.endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(converted.file_id.as_deref(), Some("file_1"));
    assert_eq!(converted.custom_llm_provider, "openai");
    assert_eq!(converted.litellm_call_id.as_deref(), Some("call_1"));
    assert_eq!(converted.timeout, None);
    assert_eq!(converted.mock_response, None);
}

#[test]
fn request_endpoint_call_keeps_the_mock_and_the_timeout() {
    let call = match parse::<Arguments>(
        r#"{"container_id": "cntr_1", "timeout": 2, "mock_response": {"id": "f"}}"#,
    ) {
        Ok(call) => call,
        Err(error) => panic!("the request should parse: {error}"),
    };
    let converted = match call.endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(converted.container_id.as_deref(), Some("cntr_1"));
    assert_eq!(converted.timeout, Some(Duration::from_secs(2)));
    let mock = IndexMap::from([(String::from("id"), Json::Text(String::from("f")))]);
    assert_eq!(converted.mock_response, Some(Json::Object(mock)));
}

#[test]
fn request_absent_or_null_call_id_answers_none() {
    let absent = match arguments("{}").endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(absent.litellm_call_id, None);
    let null = match arguments(r#"{"litellm_call_id": null}"#).endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(null.litellm_call_id, None);
}

#[rstest]
#[case::integer_zero(r#"{"timeout": 0}"#)]
#[case::float_zero(r#"{"timeout": 0.0}"#)]
#[case::negative_float(r#"{"timeout": -0.5}"#)]
#[case::negative_integer(r#"{"timeout": -1}"#)]
fn request_zero_and_negative_timeouts_read_as_absent(#[case] text: &str) {
    let converted = match arguments(text).endpoint_call(None) {
        Ok(converted) => converted,
        Err(error) => panic!("the call should convert: {error}"),
    };
    assert_eq!(converted.timeout, None);
}

#[test]
fn error_query_value_refuses_a_boolean() {
    let call = arguments(r#"{"extra_query": {"flag": true}}"#);
    let Err(error) = query_values(call.extra_query.as_ref()) else {
        panic!("a boolean is not a query value");
    };
    assert!(matches!(error, ConvertError::QueryValue { name } if name == "flag"));
}

#[test]
fn error_missing_file() {
    let call = arguments(r#"{"container_id": "cntr_1"}"#);
    let Err(error) = call.upload(&mut Vec::new()) else {
        panic!("a request without a file cannot upload");
    };
    assert!(matches!(error, ConvertError::MissingFile));
}

#[test]
fn error_unknown_blob() {
    let call = arguments(r#"{"file": {"$file": {"name": "f", "blob": 2, "content_type": null}}}"#);
    let Err(error) = call.upload(&mut [Vec::new()]) else {
        panic!("a marker cannot name a blob the call did not send");
    };
    assert!(matches!(error, ConvertError::File));
}

#[test]
fn error_call_id_of_the_wrong_type() {
    let call = arguments(r#"{"litellm_call_id": 3}"#);
    let Err(error) = call.endpoint_call(None) else {
        panic!("a number is not a call id");
    };
    assert!(matches!(error, ConvertError::Field { name } if name == "litellm_call_id"));
}

#[test]
fn error_parse_of_something_that_is_not_json() {
    let Err(error) = parse::<Arguments>("not json") else {
        panic!("the text is not JSON");
    };
    assert!(matches!(error, ConvertError::Json { .. }));
}
