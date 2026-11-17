#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_http::user_url::error::SsrfError;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use super::{
    HostedVllmVideoError, VLLM_OMNI_VIDEO_PARAMS, VideoFormRequest, get_complete_url,
    get_supported_openai_params, map_openai_params, transform_video_create_request,
    validate_environment,
};
use crate::hosted_vllm_media::{API_BASE_ENV, API_KEY_ENV};
use crate::openai_video_identifiers::multipart::MediaBody;
use crate::videos::VideoOptions;
use crate::videos::config::SUPPORTED_PARAMS;
use crate::videos::create::VideoCreate;

type Environment = BTreeMap<String, String>;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("the case is JSON")
}

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

fn allowed(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|entry| (*entry).to_owned()).collect()
}

fn build(params: Value, allowed_hosts: &[String]) -> Result<Value, HostedVllmVideoError> {
    let fields = options(params);
    let request = VideoFormRequest {
        params: &fields,
        allowed_hosts,
    };
    transform_video_create_request("model", "prompt", "http://host/v1/videos", request, None)
        .map(|written| Value::Object(written.fields))
}

fn form(params: Value) -> Value {
    build(params, &[]).expect("the form builds")
}

fn form_error(params: Value) -> HostedVllmVideoError {
    build(params, &[]).expect_err("the form must not build")
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
    let mapped = map_openai_params(&create(value(
        r#"{
            "seconds": "8",
            "size": "720x1280",
            "user": "someone",
            "characters": [{"name": "Ada"}],
            "custom": 1,
            "nulled": null
        }"#,
    )));
    assert_eq!(
        Value::Object(mapped),
        value(
            r#"{
                "seconds": "8",
                "size": "720x1280",
                "user": "someone",
                "characters": [{"name": "Ada"}],
                "custom": 1
            }"#,
        )
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
        form(value(
            r#"{
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
                "skipped": null
            }"#,
        )),
        value(
            r#"{
                "model": "model",
                "prompt": "prompt",
                "width": "8",
                "generate_sound": "true",
                "aspect_ratio": "16:9"
            }"#,
        )
    );
}

#[test]
fn the_form_renders_bools_and_containers_as_json_text() {
    assert_eq!(
        form(value(
            r#"{
                "enable_frame_interpolation": false,
                "extra_params": {"a": 1},
                "lora": ["first", "second"]
            }"#,
        )),
        value(
            r#"{
                "model": "model",
                "prompt": "prompt",
                "enable_frame_interpolation": "false",
                "extra_params": "{\"a\":1}",
                "lora": "[\"first\",\"second\"]"
            }"#,
        )
    );
}

#[test]
fn the_form_echoes_the_api_base_and_adds_the_reference_file() {
    let params = options(value("{}"));
    let request = VideoFormRequest {
        params: &params,
        allowed_hosts: &[],
    };
    let form = transform_video_create_request(
        "model",
        "prompt",
        "http://host/v1/videos",
        request,
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
#[case(r#"{"image_reference": {"image_url": "http://169.254.169.254/latest"}}"#)]
#[case(r#"{"video_reference": {"video_url": "http://10.0.0.1/clip.mp4"}}"#)]
#[case(r#"{"audio_reference": {"audio_url": "http://127.0.0.1/clip.wav"}}"#)]
#[case(r#"{"image_reference": [{"image_url": "http://192.168.0.1/x.png"}]}"#)]
#[case(r#"{"image_reference": "http://169.254.169.254/latest"}"#)]
#[case(r#"{"image_reference": {"image_url": "http://localhost:8000/x.png"}}"#)]
#[case(r#"{"image_reference": {"image_url": "http://vllm.localhost/x.png"}}"#)]
fn a_media_reference_to_an_internal_destination_is_refused(#[case] text: &str) {
    assert!(matches!(
        form_error(value(text)),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}

#[test]
fn an_ipv6_loopback_reference_is_refused() {
    assert!(matches!(
        form_error(value(
            r#"{"image_reference": {"image_url": "http://[::1]/x.png"}}"#
        )),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}

#[rstest]
#[case("http://127.0.0.1:8000/x.png", &["127.0.0.1:8000"])]
#[case("http://127.0.0.1/x.png", &["127.0.0.1"])]
#[case("http://127.0.0.1/x.png", &["127.0.0.1:80"])]
#[case("https://127.0.0.1/x.png", &["127.0.0.1:443"])]
#[case("http://localhost:8000/x.png", &["localhost"])]
#[case("http://[::1]:8000/x.png", &["[::1]:8000"])]
#[case("http://[::ffff:127.0.0.1]:8000/x.png", &["[::FFFF:127.0.0.1]:8000"])]
#[case("http://127.0.0.1/x.png", &["127.0.0.1."])]
#[case("http://[::1]:8000/x.png", &["[::1]"])]
fn a_media_reference_to_an_allowlisted_host_is_kept(#[case] url: &str, #[case] entries: &[&str]) {
    let params = value(&format!(
        r#"{{"image_reference": {{"image_url": "{url}"}}}}"#
    ));
    let written = build(params, &allowed(entries)).expect("an allowlisted host is accepted");
    let expected = value(&format!(r#"{{"image_url": "{url}"}}"#)).to_string();
    assert_eq!(
        written.get("image_reference"),
        Some(&Value::String(expected))
    );
}

#[test]
fn an_allowlist_entry_for_another_port_leaves_the_refusal() {
    let params = value(r#"{"image_reference": {"image_url": "http://127.0.0.1:8000/x.png"}}"#);
    assert!(matches!(
        build(params, &allowed(&["127.0.0.1:8001"])),
        Err(HostedVllmVideoError::UnsafeUrl(
            SsrfError::BlockedAddress { .. }
        ))
    ));
}

#[rstest]
#[case("ftp://files/x.png")]
#[case("file:///etc/passwd")]
fn a_media_reference_with_another_scheme_is_refused(#[case] url: &str) {
    let params = value(&format!(
        r#"{{"image_reference": {{"image_url": "{url}"}}}}"#
    ));
    let HostedVllmVideoError::UnsafeUrl(SsrfError::SchemeNotAllowed { scheme }) =
        form_error(params)
    else {
        panic!("a refused scheme must be a SchemeNotAllowed error");
    };
    assert_eq!(url.split(':').next(), Some(scheme.as_str()));
}

#[test]
fn a_media_reference_without_a_hostname_is_refused() {
    assert!(matches!(
        form_error(value(r#"{"image_reference": {"image_url": "http://"}}"#)),
        HostedVllmVideoError::UnsafeUrl(SsrfError::NoHostname)
    ));
}

#[rstest]
#[case(r#"{"image_reference": {"image_url": "data:image/png;base64,AA"}}"#)]
#[case(r#"{"image_reference": {"image_url": ""}}"#)]
#[case(r#"{"image_reference": {"image_url": "http://8.8.8.8/x.png"}}"#)]
#[case(r#"{"image_reference": {"image_url": "https://8.8.8.8/x.png"}}"#)]
#[case(r#"{"image_reference": {"image_url": "http://media.example/x.png"}}"#)]
#[case(r#"{"image_reference": "data:image/png;base64,AA"}"#)]
fn a_safe_media_reference_is_kept(#[case] text: &str) {
    let written = form(value(text));
    assert!(written.get("image_reference").is_some());
}

#[test]
fn a_reference_item_checks_only_its_own_url_key() {
    let written = form(value(
        r#"{
            "image_reference": [
                {"image_url": "http://8.8.8.8/x.png"},
                {"video_url": "http://169.254.169.254/x.mp4"}
            ]
        }"#,
    ));
    assert_eq!(
        written.get("image_reference"),
        Some(&Value::String(String::from(
            r#"[{"image_url":"http://8.8.8.8/x.png"},{"video_url":"http://169.254.169.254/x.mp4"}]"#
        )))
    );
}

#[test]
fn a_field_outside_the_three_references_is_not_checked() {
    let written = form(value(
        r#"{"width": "http://169.254.169.254/x", "negative_prompt": "http://10.0.0.1"}"#,
    ));
    assert_eq!(
        written.get("width"),
        Some(&Value::String(String::from("http://169.254.169.254/x")))
    );
}

#[test]
fn a_broken_json_reference_is_refused() {
    assert!(matches!(
        form_error(value(r#"{"image_reference": "{\"image_url\": "}"#)),
        HostedVllmVideoError::MediaJson { .. }
    ));
}

#[rstest]
#[case("http://8.8.8.8:bad/x.png")]
#[case("http://[::1/x")]
fn a_media_reference_the_url_parser_refuses_is_an_error(#[case] url: &str) {
    let params = value(&format!(
        r#"{{"image_reference": {{"image_url": "{url}"}}}}"#
    ));
    assert!(matches!(
        form_error(params),
        HostedVllmVideoError::UrlParse { .. }
    ));
}

#[test]
fn a_json_reference_given_as_text_is_checked() {
    assert!(matches!(
        form_error(value(
            r#"{"image_reference": "{\"image_url\": \"http://10.0.0.1/x\"}"}"#
        )),
        HostedVllmVideoError::UnsafeUrl(SsrfError::BlockedAddress { .. })
    ));
}
