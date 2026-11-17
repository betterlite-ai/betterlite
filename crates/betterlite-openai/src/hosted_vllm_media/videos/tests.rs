#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_http::user_url::error::SsrfError;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{
    HostedVllmVideoError, VLLM_OMNI_VIDEO_PARAMS, get_complete_url, get_supported_openai_params,
    map_openai_params, transform_video_create_request, validate_environment,
};
use crate::hosted_vllm_media::{API_BASE_ENV, API_KEY_ENV};
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::videos::VideoOptions;
use crate::videos::config::SUPPORTED_PARAMS;
use crate::videos::create::VideoCreate;

type Environment = BTreeMap<String, String>;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn environment(entries: &[(&str, &str)]) -> Environment {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn options(value: Value) -> VideoOptions {
    let Value::Object(fields) = value else {
        panic!("test options must be an object");
    };
    fields
}

fn create(value: Value) -> VideoCreate {
    serde_json::from_value(value).expect("the create options must decode")
}

fn form(params: Value) -> Value {
    let params = options(params);
    let form =
        transform_video_create_request("model", "prompt", "http://host/v1/videos", &params, None)
            .expect("the form builds");
    Value::Object(form.fields)
}

fn form_error(params: Value) -> HostedVllmVideoError {
    let params = options(params);
    let built =
        transform_video_create_request("model", "prompt", "http://host/v1/videos", &params, None);
    match built {
        Ok(_) => panic!("the form must not build"),
        Err(error) => error,
    }
}

#[test]
fn supported_params_are_the_openai_eight_then_the_vllm_omni_fields() {
    let supported = get_supported_openai_params();
    assert_eq!(supported.len(), 32);
    assert_eq!(supported.get(..8), Some(SUPPORTED_PARAMS.as_slice()));
    assert_eq!(supported.get(8..), Some(VLLM_OMNI_VIDEO_PARAMS.as_slice()));
}

#[test]
fn mapped_params_keep_every_set_option_and_drop_the_nulls() {
    let mapped = map_openai_params(&create(json!({
        "seconds": "8",
        "size": "720x1280",
        "user": "someone",
        "characters": [{"name": "Ada"}],
        "custom": 1,
        "nulled": null,
    })));
    assert_eq!(
        Value::Object(mapped),
        json!({
            "seconds": "8",
            "size": "720x1280",
            "user": "someone",
            "characters": [{"name": "Ada"}],
            "custom": 1,
        })
    );
}

#[test]
fn headers_prefer_the_call_key_over_the_model_key() {
    assert_eq!(
        validate_environment(
            headers(&[("Authorization", "Bearer caller")]),
            Some(&secret("model-key")),
            Some(&secret("call-key")),
            &environment(&[]),
        ),
        headers(&[("Authorization", "Bearer call-key")])
    );
}

#[test]
fn headers_fall_through_an_empty_call_key_to_the_model_key() {
    assert_eq!(
        validate_environment(
            headers(&[]),
            Some(&secret("model-key")),
            Some(&secret("")),
            &environment(&[]),
        )
        .get("Authorization"),
        Some(&String::from("Bearer model-key"))
    );
}

#[test]
fn headers_fall_through_to_the_environment_and_then_the_placeholder() {
    assert_eq!(
        validate_environment(
            headers(&[]),
            None,
            None,
            &environment(&[(API_KEY_ENV, "env-key")]),
        )
        .get("Authorization"),
        Some(&String::from("Bearer env-key"))
    );
    assert_eq!(
        validate_environment(headers(&[]), None, None, &environment(&[])).get("Authorization"),
        Some(&String::from("Bearer fake-api-key"))
    );
}

#[test]
fn complete_url_appends_the_videos_path_under_v1() {
    assert_eq!(
        get_complete_url(Some("http://host/v1/"), &environment(&[])).expect("a base is set"),
        "http://host/v1/videos"
    );
    assert_eq!(
        get_complete_url(None, &environment(&[(API_BASE_ENV, "http://env")]))
            .expect("the environment names a base"),
        "http://env/v1/videos"
    );
}

#[test]
fn complete_url_requires_a_base() {
    assert!(matches!(
        get_complete_url(None, &environment(&[])),
        Err(HostedVllmVideoError::MissingApiBase)
    ));
}

#[test]
fn the_form_writes_model_and_prompt_then_every_remaining_option() {
    assert_eq!(
        form(json!({
            "width": 8,
            "generate_sound": true,
            "aspect_ratio": "16:9",
            "extra_headers": {"x": "1"},
            "extra_query": {"y": "2"},
            "extra_body": {"z": 3},
            "timeout": 30,
            "custom_llm_provider": "hosted_vllm",
            "input_reference": "http://10.0.0.1/x.png",
            "characters": [{"name": "Ada"}],
            "model": "other",
            "prompt": "other",
            "skipped": null,
        })),
        json!({
            "model": "model",
            "prompt": "prompt",
            "width": "8",
            "generate_sound": "true",
            "aspect_ratio": "16:9",
        })
    );
}

#[test]
fn the_form_renders_bools_and_containers_as_json_text() {
    assert_eq!(
        form(json!({
            "enable_frame_interpolation": false,
            "extra_params": {"a": 1},
            "lora": ["first", "second"],
        })),
        json!({
            "model": "model",
            "prompt": "prompt",
            "enable_frame_interpolation": "false",
            "extra_params": "{\"a\":1}",
            "lora": "[\"first\",\"second\"]",
        })
    );
}

#[test]
fn the_form_echoes_the_api_base_and_adds_the_reference_file() {
    let params = options(json!({}));
    let form = transform_video_create_request(
        "model",
        "prompt",
        "http://host/v1/videos",
        &params,
        Some(MediaBody::Bytes(PNG)),
    )
    .expect("the form builds");
    assert_eq!(form.api_base, "http://host/v1/videos");
    assert_eq!(form.files.len(), 1);
    let part = form.files.first().expect("one reference file");
    assert_eq!(part.field, "input_reference");
    assert_eq!(part.filename, "input_reference.png");
    assert_eq!(part.content_type, "image/png");
}

#[rstest]
#[case(json!({"image_reference": {"image_url": "http://169.254.169.254/latest"}}))]
#[case(json!({"video_reference": {"video_url": "http://10.0.0.1/clip.mp4"}}))]
#[case(json!({"audio_reference": {"audio_url": "http://127.0.0.1/clip.wav"}}))]
#[case(json!({"image_reference": [{"image_url": "http://192.168.0.1/x.png"}]}))]
#[case(json!({"image_reference": "http://169.254.169.254/latest"}))]
#[case(json!({"image_reference": {"image_url": "http://localhost:8000/x.png"}}))]
#[case(json!({"image_reference": {"image_url": "http://vllm.localhost/x.png"}}))]
fn a_media_reference_to_an_internal_destination_is_refused(#[case] params: Value) {
    assert!(matches!(
        form_error(params),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}

#[test]
fn an_ipv6_loopback_reference_is_refused() {
    assert!(matches!(
        form_error(json!({"image_reference": {"image_url": "http://[::1]/x.png"}})),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}

#[rstest]
#[case("ftp://files/x.png")]
#[case("file:///etc/passwd")]
fn a_media_reference_with_another_scheme_is_refused(#[case] url: &str) {
    let error = form_error(json!({"image_reference": {"image_url": url}}));
    match error {
        HostedVllmVideoError::UnsafeUrl(SsrfError::SchemeNotAllowed { scheme }) => {
            assert_eq!(url.split(':').next(), Some(scheme.as_str()));
        }
        other => panic!("unexpected error {other}"),
    }
}

#[test]
fn a_media_reference_without_a_hostname_is_refused() {
    assert!(matches!(
        form_error(json!({"image_reference": {"image_url": "http://"}})),
        HostedVllmVideoError::UnsafeUrl(SsrfError::NoHostname)
    ));
}

#[rstest]
#[case(json!({"image_reference": {"image_url": "data:image/png;base64,AA"}}))]
#[case(json!({"image_reference": {"image_url": ""}}))]
#[case(json!({"image_reference": {"image_url": "http://8.8.8.8/x.png"}}))]
#[case(json!({"image_reference": {"image_url": "https://8.8.8.8/x.png"}}))]
#[case(json!({"image_reference": {"image_url": "http://media.example/x.png"}}))]
#[case(json!({"image_reference": "data:image/png;base64,AA"}))]
fn a_safe_media_reference_is_kept(#[case] params: Value) {
    let written = form(params);
    assert!(written.get("image_reference").is_some());
}

#[test]
fn a_reference_item_checks_only_its_own_url_key() {
    let written = form(json!({
        "image_reference": [
            {"image_url": "http://8.8.8.8/x.png"},
            {"video_url": "http://169.254.169.254/x.mp4"},
        ],
    }));
    assert_eq!(
        written.get("image_reference"),
        Some(&json!(
            "[{\"image_url\":\"http://8.8.8.8/x.png\"},{\"video_url\":\"http://169.254.169.254/x.mp4\"}]"
        ))
    );
}

#[test]
fn a_field_outside_the_three_references_is_not_checked() {
    let written =
        form(json!({"width": "http://169.254.169.254/x", "negative_prompt": "http://10.0.0.1"}));
    assert_eq!(
        written.get("width"),
        Some(&json!("http://169.254.169.254/x"))
    );
}

#[test]
fn a_broken_json_reference_is_refused() {
    assert!(matches!(
        form_error(json!({"image_reference": "{\"image_url\": "})),
        HostedVllmVideoError::MediaJson { .. }
    ));
}

#[rstest]
#[case("http://8.8.8.8:bad/x.png")]
#[case("http://[::1/x")]
fn a_media_reference_the_url_parser_refuses_is_an_error(#[case] url: &str) {
    assert!(matches!(
        form_error(json!({"image_reference": {"image_url": url}})),
        HostedVllmVideoError::UrlParse { .. }
    ));
}

#[test]
fn a_json_reference_given_as_text_is_checked() {
    assert!(matches!(
        form_error(json!({"image_reference": "{\"image_url\": \"http://10.0.0.1/x\"}"})),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}
