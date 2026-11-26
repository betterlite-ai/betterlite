#![cfg(test)]

use std::time::Duration;

use betterlite_core::api::bfl::edit::media::SuppliedMedia;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::fields::drop_params;
use super::{
    ConvertError, Reference, SettingsWire, VideoCallWire, content_request, generation_request,
    list_request, parse, remix_request, status_request,
};

fn wire(arguments: &str) -> VideoCallWire {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, &mut []).unwrap().0
}

fn reference_wire(arguments: &str, blobs: &mut [Vec<u8>]) -> (VideoCallWire, Option<Reference>) {
    let payload = format!("{{\"arguments\":{arguments}}}");
    parse(&payload, blobs).unwrap()
}

#[test]
fn request_generation_arguments_reach_the_call() {
    let parsed = wire(
        r#"{"prompt": "a cat", "model": "sora-2", "seconds": "8", "size": "720x1280",
            "user": "u-1", "timeout": 12.5, "custom_llm_provider": "openai",
            "extra_headers": {"x-one": "1"}, "extra_query": {"api-version": "2"},
            "extra_body": {"watermark": "yes"}}"#,
    );
    let (request, _media) = generation_request(&parsed, None).unwrap();
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
    let (request, _media) =
        generation_request(&wire(r#"{"prompt": "p", "seconds": 8}"#), None).unwrap();
    assert_eq!(request.seconds.as_deref(), Some("8"));
}

#[test]
fn request_generation_timeout_null_is_none() {
    let (request, _media) =
        generation_request(&wire(r#"{"prompt": "p", "timeout": null}"#), None).unwrap();
    assert_eq!(request.timeout, None);
}

#[test]
fn request_generation_loose_keywords_stay_in_kwargs() {
    let (request, _media) = generation_request(
        &wire(
            r#"{"prompt": "p", "api_base": "http://example.test/v1", "api_key": "k",
                "mock_response": "m", "litellm_call_id": "c"}"#,
        ),
        None,
    )
    .unwrap();
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
    let (parsed, media) = reference_wire(
        r#"{"prompt": "p", "input_reference": {"$file": {"blob": 0, "name": "ref.png", "content_type": "image/png"}}}"#,
        &mut blobs,
    );
    let (request, media) = generation_request(&parsed, media).unwrap();
    let reference = media.unwrap();
    assert_eq!(reference.name.as_deref(), Some("ref.png"));
    assert_eq!(reference.content, b"PNG");
    assert_eq!(
        request.input_reference,
        Some(SuppliedMedia::Bytes(b"PNG".to_vec()))
    );
}

#[test]
fn request_generation_default_file_name_is_unnamed() {
    let mut blobs = vec![b"PNG".to_vec()];
    let (parsed, media) = reference_wire(
        r#"{"prompt": "p", "input_reference": {"$file": {"blob": 0}}}"#,
        &mut blobs,
    );
    let (_request, media) = generation_request(&parsed, media).unwrap();
    assert_eq!(media.unwrap().name, None);
}

#[test]
fn request_content_arguments_reach_the_call() {
    let request = content_request(&wire(
        r#"{"video_id": "video_123", "timeout": 2.5, "custom_llm_provider": "openai",
            "variant": "thumbnail", "extra_headers": {"x-one": "1"}}"#,
    ))
    .unwrap();
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
    let request = content_request(&wire(r#"{"video_id": "v", "timeout": null}"#)).unwrap();
    assert_eq!(request.timeout, None);
}

#[test]
fn request_remix_arguments_reach_the_call() {
    let request = remix_request(&wire(
        r#"{"video_id": "video_123", "prompt": "again", "custom_llm_provider": "openai",
            "api_key": "sk-1", "api_base": "http://example.test/v1", "api_version": "2024-01",
            "extra_headers": {"x-one": "1"}, "extra_body": {"seconds": "4"},
            "mock_response": {"id": "v"}, "litellm_metadata": {"user_api_key": "k"},
            "litellm_call_id": "call-1", "user": "u-1", "timeout": 30, "litellm_trace_id": "t-1"}"#,
    ))
    .unwrap();
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
    let request = list_request(&wire(
        r#"{"after": "video_1", "limit": 3, "order": "asc", "api_key": "sk-1",
            "timeout": 5, "custom_llm_provider": "openai"}"#,
    ))
    .unwrap();
    assert_eq!(request.after.as_deref(), Some("video_1"));
    assert_eq!(request.limit, Some(3));
    assert_eq!(request.order.as_deref(), Some("asc"));
    assert_eq!(request.timeout, Some(5.0));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(request.api_key.as_ref().unwrap().expose_secret(), "sk-1");
}

#[test]
fn request_list_limit_zero_is_kept() {
    let request = list_request(&wire(r#"{"limit": 0}"#)).unwrap();
    assert_eq!(request.limit, Some(0));
}

#[test]
fn request_status_absent_timeout_takes_the_request_timeout() {
    let request = status_request(&wire(r#"{"video_id": "v"}"#)).unwrap();
    assert_eq!(request.timeout, Some(Duration::ZERO));
}

#[test]
fn request_status_null_timeout_takes_the_request_timeout() {
    let request = status_request(&wire(r#"{"video_id": "v", "timeout": null}"#)).unwrap();
    assert_eq!(request.timeout, Some(Duration::ZERO));
}

#[test]
fn request_status_given_timeout_is_kept() {
    let request = status_request(&wire(r#"{"video_id": "v", "timeout": 12.5}"#)).unwrap();
    assert_eq!(request.timeout, Some(Duration::from_secs_f64(12.5)));
}

#[test]
fn request_drop_params_reads_the_argument_or_the_settings() {
    let settings = SettingsWire {
        api_key: None,
        openai_key: None,
        drop_params: Some(true),
    };
    let arguments = wire(r#"{"prompt": "p"}"#).arguments;
    assert!(drop_params(&arguments, &settings).unwrap());
    let folded = SettingsWire {
        api_key: None,
        openai_key: None,
        drop_params: Some(false),
    };
    let arguments = wire(r#"{"drop_params": true}"#).arguments;
    assert!(drop_params(&arguments, &folded).unwrap());
}

#[test]
fn error_generation_without_a_prompt_names_the_field() {
    let error = generation_request(&wire(r#"{}"#), None).err().unwrap();
    assert!(matches!(error, ConvertError::Field { .. }));
    assert_eq!(error.to_string(), "prompt must be a string");
}

#[test]
fn error_generation_seconds_of_the_wrong_type_names_the_field() {
    let error = generation_request(&wire(r#"{"prompt": "p", "seconds": true}"#), None)
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "seconds must be a string");
}

#[test]
fn error_timeout_that_is_not_a_number_names_the_field() {
    let error = content_request(&wire(r#"{"video_id": "v", "timeout": "soon"}"#))
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "timeout must be a number of seconds");
}

#[test]
fn error_list_limit_that_is_not_an_integer_names_the_field() {
    let error = list_request(&wire(r#"{"limit": 1.5}"#)).err().unwrap();
    assert_eq!(error.to_string(), "limit must be an integer");
}

#[test]
fn error_header_value_that_is_not_a_string_names_the_field() {
    let error = remix_request(&wire(
        r#"{"video_id": "v", "prompt": "p", "extra_headers": {"x-one": 1}}"#,
    ))
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "extra_headers must be a map of strings");
}

#[test]
fn error_input_reference_with_an_unknown_blob_is_reported() {
    let mut blobs = Vec::new();
    let error = reference_wire(
        r#"{"prompt": "p", "input_reference": {"$file": {"blob": 0}}}"#,
        &mut blobs,
    )
    .err()
    .unwrap();
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
