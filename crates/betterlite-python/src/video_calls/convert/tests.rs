#![cfg(test)]

use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_core::api::video_content_call::VideoContentRequest;
use betterlite_core::api::video_generation_call::VideoGenerationRequest;
use betterlite_core::api::video_list_call::VideoListCall;
use betterlite_core::api::video_remix_call::VideoRemixCall;
use betterlite_core::api::video_status_call::VideoStatusRequest;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::{ConvertError, Operation, Requests, VideoCallWire, drop_params, parse, request};

fn wire(arguments: &str) -> VideoCallWire {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, &mut []).unwrap().0
}

fn build(operation: Operation, arguments: &str) -> Result<Requests, ConvertError> {
    request(operation, &wire(arguments), None)
}

fn generation(arguments: &str) -> VideoGenerationRequest {
    build(Operation::Generate, arguments)
        .unwrap()
        .generation
        .unwrap()
}

fn content(arguments: &str) -> VideoContentRequest {
    build(Operation::Content, arguments)
        .unwrap()
        .content
        .unwrap()
}

fn remix(arguments: &str) -> VideoRemixCall {
    build(Operation::Remix, arguments).unwrap().remix.unwrap()
}

fn list(arguments: &str) -> VideoListCall {
    build(Operation::List, arguments).unwrap().list.unwrap()
}

fn status(arguments: &str) -> VideoStatusRequest {
    build(Operation::Status, arguments).unwrap().status.unwrap()
}

#[test]
fn request_generation_arguments_reach_the_call() {
    let request = generation(
        r#"{"prompt": "a cat", "model": "sora-2", "seconds": "8", "size": "720x1280",
            "user": "u-1", "timeout": 12.5, "custom_llm_provider": "openai",
            "extra_headers": {"x-one": "1"}, "extra_query": {"api-version": "2"},
            "extra_body": {"watermark": "yes"}}"#,
    );
    assert_eq!(request.prompt, "a cat");
    assert_eq!(request.model.as_deref(), Some("sora-2"));
    assert_eq!(request.seconds.as_deref(), Some("8"));
    assert_eq!(request.size.as_deref(), Some("720x1280"));
    assert_eq!(request.user.as_deref(), Some("u-1"));
    assert_eq!(request.timeout, Some(12.5));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        request.extra_query.unwrap().get("api-version"),
        Some(&Json::Text(String::from("2")))
    );
    assert_eq!(
        request.extra_body.unwrap().get("watermark"),
        Some(&Json::Text(String::from("yes")))
    );
    assert!(request.kwargs.is_empty());
}

#[test]
fn request_generation_seconds_integer_is_sent_as_given() {
    assert_eq!(
        generation(r#"{"prompt": "p", "seconds": 8}"#)
            .seconds
            .as_deref(),
        Some("8")
    );
}

#[test]
fn request_generation_timeout_null_is_none() {
    assert_eq!(
        generation(r#"{"prompt": "p", "timeout": null}"#).timeout,
        None
    );
}

#[test]
fn request_generation_loose_keywords_stay_in_kwargs() {
    let request = generation(
        r#"{"prompt": "p", "api_base": "http://example.test/v1", "api_key": "k",
            "mock_response": "m", "litellm_call_id": "c"}"#,
    );
    assert_eq!(
        request.kwargs.get("api_base"),
        Some(&Json::Text(String::from("http://example.test/v1")))
    );
    assert_eq!(
        request.kwargs.get("api_key"),
        Some(&Json::Text(String::from("k")))
    );
    assert_eq!(
        request.kwargs.get("mock_response"),
        Some(&Json::Text(String::from("m")))
    );
    assert_eq!(
        request.kwargs.get("litellm_call_id"),
        Some(&Json::Text(String::from("c")))
    );
}

#[test]
fn request_generation_input_reference_bytes_carry_their_name() {
    let mut blobs = vec![b"PNG".to_vec()];
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0, "name": "ref.png", "content_type": "image/png"}}}}"#;
    let (wire, media) = parse(payload, &mut blobs).unwrap();
    let media = media.unwrap();
    assert_eq!(media.name.as_deref(), Some("ref.png"));
    assert_eq!(media.content, b"PNG".to_vec());
    let request = request(Operation::Generate, &wire, Some(media))
        .unwrap()
        .generation
        .unwrap();
    assert_eq!(
        request.input_reference,
        Some(SuppliedMedia::Bytes(b"PNG".to_vec()))
    );
}

#[test]
fn request_generation_default_file_name_is_unnamed() {
    let mut blobs = vec![b"PNG".to_vec()];
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0}}}}"#;
    let (wire, media) = parse(payload, &mut blobs).unwrap();
    let media = media.unwrap();
    assert_eq!(media.name, None);
    assert_eq!(media.content, b"PNG".to_vec());
    let request = request(Operation::Generate, &wire, Some(media))
        .unwrap()
        .generation
        .unwrap();
    assert_eq!(
        request.input_reference,
        Some(SuppliedMedia::Bytes(b"PNG".to_vec()))
    );
}

#[test]
fn request_content_arguments_reach_the_call() {
    let request = content(
        r#"{"video_id": "video_123", "timeout": 2.5, "custom_llm_provider": "openai",
            "variant": "thumbnail", "extra_headers": {"x-one": "1"}}"#,
    );
    assert_eq!(request.video_id, "video_123");
    assert_eq!(request.timeout, Some(Duration::from_secs_f64(2.5)));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.variant.as_deref(), Some("thumbnail"));
    assert_eq!(
        request
            .extra_headers
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
}

#[test]
fn request_content_timeout_null_stays_none() {
    assert_eq!(
        content(r#"{"video_id": "v", "timeout": null}"#).timeout,
        None
    );
}

#[test]
fn request_remix_arguments_reach_the_call() {
    let request = remix(
        r#"{"video_id": "video_123", "prompt": "again", "custom_llm_provider": "openai",
            "api_key": "sk-1", "api_base": "http://example.test/v1", "api_version": "2024-01",
            "extra_headers": {"x-one": "1"}, "extra_body": {"seconds": "4"},
            "mock_response": {"id": "v"}, "litellm_metadata": {"user_api_key": "k"},
            "litellm_call_id": "call-1", "user": "u-1", "timeout": 30, "litellm_trace_id": "t-1"}"#,
    );
    assert_eq!(request.video_id, "video_123");
    assert_eq!(request.prompt, "again");
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_base.as_deref(), Some("http://example.test/v1"));
    assert_eq!(request.api_version.as_deref(), Some("2024-01"));
    assert_eq!(request.user.as_deref(), Some("u-1"));
    assert_eq!(request.timeout, Some(30.0));
    assert_eq!(
        request
            .extra_headers
            .as_ref()
            .unwrap()
            .get("x-one")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        request.extra_body.as_ref().unwrap().get("seconds"),
        Some(&Json::Text(String::from("4")))
    );
    assert_eq!(
        request.mock_response,
        Some(Json::Object(IndexMap::from_iter([(
            String::from("id"),
            Json::Text(String::from("v")),
        )])))
    );
    assert!(request.litellm_metadata.is_some());
    assert_eq!(request.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(
        request.extra.get("litellm_trace_id"),
        Some(&Json::Text(String::from("t-1")))
    );
    assert_eq!(request.api_key.as_ref().unwrap().expose_secret(), "sk-1");
}

#[test]
fn request_list_arguments_reach_the_call() {
    let request = list(
        r#"{"after": "video_1", "limit": 3, "order": "asc", "api_key": "sk-1",
            "timeout": 5, "custom_llm_provider": "openai"}"#,
    );
    assert_eq!(request.after.as_deref(), Some("video_1"));
    assert_eq!(request.limit, Some(3));
    assert_eq!(request.order.as_deref(), Some("asc"));
    assert_eq!(request.timeout, Some(5.0));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_key.as_ref().unwrap().expose_secret(), "sk-1");
}

#[test]
fn request_list_limit_zero_is_kept() {
    assert_eq!(list(r#"{"limit": 0}"#).limit, Some(0));
}

#[test]
fn request_status_absent_timeout_takes_the_request_timeout() {
    assert_eq!(status(r#"{"video_id": "v"}"#).timeout, Some(Duration::ZERO));
}

#[test]
fn request_status_null_timeout_takes_the_request_timeout() {
    assert_eq!(
        status(r#"{"video_id": "v", "timeout": null}"#).timeout,
        Some(Duration::ZERO)
    );
}

#[test]
fn request_status_given_timeout_is_kept() {
    assert_eq!(
        status(r#"{"video_id": "v", "timeout": 12.5}"#).timeout,
        Some(Duration::from_secs_f64(12.5))
    );
}

#[test]
fn request_drop_params_reads_the_argument_or_the_settings() {
    assert!(drop_params(&wire(r#"{"drop_params": true}"#)).unwrap());
    let payload = r#"{"arguments": {"prompt": "p"}, "settings": {"drop_params": true}}"#;
    let (wire, _media) = parse(payload, &mut []).unwrap();
    assert!(drop_params(&wire).unwrap());
    let payload = r#"{"arguments": {"prompt": "p"}, "settings": {"drop_params": false}}"#;
    let (wire, _media) = parse(payload, &mut []).unwrap();
    assert!(!drop_params(&wire).unwrap());
}

#[test]
fn error_generation_without_a_prompt_names_the_field() {
    let error = build(Operation::Generate, r#"{}"#).err().unwrap();
    assert!(matches!(error, ConvertError::Field { .. }));
    assert_eq!(error.to_string(), "prompt must be a string");
}

#[test]
fn error_generation_seconds_of_the_wrong_type_names_the_field() {
    let error = build(Operation::Generate, r#"{"prompt": "p", "seconds": true}"#)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "seconds must be a string");
}

#[test]
fn error_timeout_that_is_not_a_number_names_the_field() {
    let error = build(
        Operation::Content,
        r#"{"video_id": "v", "timeout": "soon"}"#,
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "timeout must be a number of seconds");
}

#[test]
fn error_list_limit_that_is_not_an_integer_names_the_field() {
    let error = build(Operation::List, r#"{"limit": 1.5}"#).err().unwrap();
    assert_eq!(error.to_string(), "limit must be an integer");
}

#[test]
fn error_header_value_that_is_not_a_string_names_the_field() {
    let error = build(
        Operation::Remix,
        r#"{"video_id": "v", "prompt": "p", "extra_headers": {"x-one": 1}}"#,
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "extra_headers must be a map of strings");
}

#[test]
fn error_input_reference_with_an_unknown_blob_is_reported() {
    let mut blobs = Vec::new();
    let payload = r#"{"arguments": {"prompt": "p", "input_reference": {"$file": {"blob": 0}}}}"#;
    let error = parse(payload, &mut blobs).err().unwrap();
    assert_eq!(
        error.to_string(),
        "the input_reference file names blob 0, which the call did not send"
    );
}

#[test]
fn error_payload_that_is_not_json_is_reported() {
    let error = parse("not json", &mut []).err().unwrap();
    assert!(
        error
            .to_string()
            .starts_with("the native request text is not JSON of the expected shape:")
    );
}
