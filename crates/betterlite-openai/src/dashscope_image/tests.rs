#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use super::error::DashscopeImageError;
use super::identity::{ImageIdentity, resolve_image_key};
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::reply::decode_dashscope_image_reply;
use super::request::transform_image_generation_request;
use super::{
    CHAT_COMPATIBLE_MODE_PATH, DEFAULT_API_BASE, DashscopeImageConfig, MISSING_KEY_MESSAGE,
};
use crate::api::image_contracts::records::{ImageData, ImageParams};
use crate::dashscope_embedding::error::DashscopeError;
use crate::openai_base_config::Headers;

fn plain_identity(env_key: Option<&str>, env_image_base: Option<&str>) -> ImageIdentity {
    ImageIdentity {
        missing_key_message: MISSING_KEY_MESSAGE.to_owned(),
        env_key: env_key.map(SecretString::from),
        fallback_key: None,
        env_image_base: env_image_base.map(str::to_owned),
        default_image_base: DEFAULT_API_BASE.to_owned(),
    }
}

fn config(env_key: Option<&str>, env_image_base: Option<&str>) -> DashscopeImageConfig {
    DashscopeImageConfig {
        identity: plain_identity(env_key, env_image_base),
    }
}

fn params(entries: &[(&str, Json)]) -> ImageParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn headers(entries: &[(&str, &str)]) -> Headers {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn image(url: &str) -> ImageData {
    ImageData {
        b64_json: None,
        revised_prompt: None,
        url: Some(url.to_owned()),
    }
}

fn reply_failure(result: Result<Vec<ImageData>, DashscopeImageError>) -> DashscopeError {
    match result {
        Err(DashscopeImageError::Reply(error)) => error,
        _ => panic!("expected a typed reply failure"),
    }
}

#[rstest]
#[case::a_256_size("256x256", "256*256")]
#[case::a_512_size("512x512", "512*512")]
#[case::a_1024_size("1024x1024", "1024*1024")]
#[case::a_landscape_size("1792x1024", "1792*1024")]
#[case::a_portrait_size("1024x1792", "1024*1792")]
#[case::a_2048_size("2048x2048", "2048*2048")]
fn map_openai_params_maps_every_table_size(#[case] openai: &str, #[case] dashscope: &str) {
    let mapped =
        map_openai_params(&params(&[("size", text(openai))]), &ImageParams::new()).unwrap();
    assert_eq!(mapped.get("size"), Some(&text(dashscope)));
}

#[rstest]
#[case::an_unlisted_size("800x600", "800*600")]
#[case::an_upper_case_x("800X600", "800X600")]
#[case::an_x_without_a_size("xlarge", "*large")]
#[case::a_size_without_an_x("auto", "auto")]
#[case::a_dashscope_size("1024*1024", "1024*1024")]
fn map_openai_params_rewrites_x_for_an_unlisted_size(
    #[case] openai: &str,
    #[case] dashscope: &str,
) {
    let mapped =
        map_openai_params(&params(&[("size", text(openai))]), &ImageParams::new()).unwrap();
    assert_eq!(mapped.get("size"), Some(&text(dashscope)));
}

#[test]
fn map_openai_params_copies_n_and_drops_every_other_key() {
    let non_default = params(&[
        ("n", Json::Number(Number::from(2_i32))),
        ("quality", text("hd")),
        ("size", text("512x512")),
    ]);
    let mapped = map_openai_params(&non_default, &ImageParams::new()).unwrap();
    assert_eq!(
        mapped,
        params(&[
            ("n", Json::Number(Number::from(2_i32))),
            ("size", text("512*512")),
        ])
    );
}

#[test]
fn map_openai_params_returns_only_the_mapped_entries() {
    let mapped = map_openai_params(
        &params(&[("n", Json::Number(Number::from(1_i32)))]),
        &params(&[("user", text("max"))]),
    )
    .unwrap();
    assert_eq!(mapped, params(&[("n", Json::Number(Number::from(1_i32)))]));
}

#[test]
fn map_openai_params_skips_a_key_the_caller_already_set() {
    let optional = params(&[
        ("n", Json::Number(Number::from(1_i32))),
        ("size", text("1024x1024")),
    ]);
    let mapped = map_openai_params(
        &params(&[
            ("n", Json::Number(Number::from(3_i32))),
            ("size", text("512x512")),
        ]),
        &optional,
    )
    .unwrap();
    assert!(mapped.is_empty());
}

#[test]
fn map_openai_params_rejects_a_size_that_is_not_text() {
    let error = map_openai_params(
        &params(&[("size", Json::Number(Number::from(1024_i32)))]),
        &ImageParams::new(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "the DashScope image generation parameter `size` must be text"
    );
}

#[test]
fn the_supported_options_are_n_and_size() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["n", "size"]);
}

#[rstest]
#[case::a_plain_base(
    Some("https://proxy.test/v1"),
    Some("https://env.test/image"),
    "https://proxy.test/v1"
)]
#[case::a_plain_base_with_a_trailing_slash(
    Some("https://proxy.test/v1/"),
    None,
    "https://proxy.test/v1/"
)]
#[case::a_chat_shaped_base(
    Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
    Some("https://env.test/image"),
    "https://env.test/image"
)]
#[case::a_chat_shaped_base_with_trailing_slashes(
    Some("https://dashscope.aliyuncs.com/compatible-mode/v1///"),
    None,
    DEFAULT_API_BASE
)]
#[case::an_empty_base(Some(""), Some("https://env.test/image"), "https://env.test/image")]
#[case::the_environment_base(None, Some("https://env.test/image"), "https://env.test/image")]
#[case::an_empty_environment_base(None, Some(""), DEFAULT_API_BASE)]
#[case::the_default(None, None, DEFAULT_API_BASE)]
fn complete_url_follows_the_family_rule(
    #[case] api_base: Option<&str>,
    #[case] env_image_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        config(None, env_image_base).complete_url(api_base),
        expected
    );
}

#[test]
fn complete_url_drops_a_chat_shaped_base_without_a_scheme() {
    let base = format!("https://host{CHAT_COMPATIBLE_MODE_PATH}");
    assert_eq!(
        config(None, Some("https://env.test/image")).complete_url(Some(&base)),
        "https://env.test/image"
    );
}

#[test]
fn validate_environment_adds_the_two_headers_and_keeps_the_caller_ones() {
    let caller = headers(&[
        ("x-trace", "abc"),
        ("Authorization", "Bearer caller"),
        ("Content-Type", "text/plain"),
    ]);
    let resolved = config(None, None)
        .validate_environment(Some("sk-arg"), &caller)
        .unwrap();
    assert_eq!(
        resolved,
        headers(&[
            ("Authorization", "Bearer sk-arg"),
            ("Content-Type", "application/json"),
            ("x-trace", "abc"),
        ])
    );
}

#[test]
fn validate_environment_takes_the_environment_key_before_the_fallback() {
    let resolved = config(Some("sk-env"), None)
        .validate_environment(None, &Headers::new())
        .unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
}

#[test]
fn validate_environment_skips_an_empty_argument_and_an_empty_environment_key() {
    let resolved = config(Some(""), None)
        .validate_environment(Some(""), &Headers::new())
        .unwrap_err();
    assert_eq!(
        resolved,
        DashscopeImageError::MissingKey {
            message: MISSING_KEY_MESSAGE.to_owned()
        }
    );
}

#[test]
fn validate_environment_falls_through_to_the_fallback_key() {
    let identity = ImageIdentity {
        fallback_key: Some(SecretString::from("sk-fallback")),
        ..plain_identity(Some(""), None)
    };
    let config = DashscopeImageConfig { identity };
    let resolved = config.validate_environment(None, &Headers::new()).unwrap();
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer sk-fallback")
    );
}

#[test]
fn validate_environment_reports_the_identity_message() {
    let error = config(None, None)
        .validate_environment(None, &Headers::new())
        .unwrap_err();
    assert_eq!(error.to_string(), MISSING_KEY_MESSAGE);
}

#[test]
fn resolve_image_key_prefers_a_non_empty_argument() {
    let identity = plain_identity(Some("sk-env"), None);
    assert_eq!(
        resolve_image_key(&identity, Some("sk-arg"))
            .unwrap()
            .expose_secret(),
        "sk-arg"
    );
    assert_eq!(
        resolve_image_key(&identity, None).unwrap().expose_secret(),
        "sk-env"
    );
}

#[test]
fn resolve_image_key_skips_an_empty_argument() {
    let identity = plain_identity(Some("sk-env"), None);
    assert_eq!(
        resolve_image_key(&identity, Some(""))
            .unwrap()
            .expose_secret(),
        "sk-env"
    );
}

#[test]
fn resolve_image_key_falls_through_to_the_fallback_and_then_the_message() {
    let identity = ImageIdentity {
        fallback_key: Some(SecretString::from("sk-fallback")),
        ..plain_identity(None, None)
    };
    assert_eq!(
        resolve_image_key(&identity, Some(""))
            .unwrap()
            .expose_secret(),
        "sk-fallback"
    );
    let message = resolve_image_key(&plain_identity(None, None), None).unwrap_err();
    assert_eq!(message.to_string(), MISSING_KEY_MESSAGE);
    let blank_fallback = ImageIdentity {
        fallback_key: Some(SecretString::from("")),
        ..plain_identity(None, None)
    };
    let blank_message = resolve_image_key(&blank_fallback, None).unwrap_err();
    assert_eq!(blank_message.to_string(), MISSING_KEY_MESSAGE);
}

#[test]
fn transform_image_generation_request_nests_the_prompt_and_copies_the_parameters() {
    let body = transform_image_generation_request(
        "qwen-image-2.0-pro",
        "a red fox",
        &params(&[
            ("size", text("1024*1024")),
            ("n", Json::Number(Number::from(1_i32))),
        ]),
    );
    let Some(Json::Object(input)) = body.get("input") else {
        panic!("expected the input object");
    };
    let Some(Json::List(messages)) = input.get("messages") else {
        panic!("expected the messages list");
    };
    assert_eq!(
        messages.first(),
        Some(&Json::Object(IndexMap::from([
            ("role".to_owned(), text("user")),
            (
                "content".to_owned(),
                Json::List(vec![Json::Object(IndexMap::from([(
                    "text".to_owned(),
                    text("a red fox"),
                )]))]),
            ),
        ])))
    );
    assert_eq!(
        body.get("parameters"),
        Some(&Json::Object(params(&[
            ("size", text("1024*1024")),
            ("n", Json::Number(Number::from(1_i32))),
        ])))
    );
    assert_eq!(body.get("model"), Some(&text("qwen-image-2.0-pro")));
}

#[test]
fn transform_image_generation_request_sends_an_empty_parameter_object() {
    let body = transform_image_generation_request("qwen-image-2.0", "a fox", &ImageParams::new());
    assert_eq!(
        body.get("parameters"),
        Some(&Json::Object(ImageParams::new()))
    );
}

#[test]
fn the_reply_collects_every_image_from_every_choice() {
    let body = br#"{"output":{"choices":[{"message":{"content":[{"image":"https://img/1.png"}]}},{"message":{"content":[{"image":"https://img/2.png"},{"image":""},{"text":"hi"}]}}]}}"#;
    let data = decode_dashscope_image_reply(body, 200, &Headers::new()).unwrap();
    assert_eq!(
        data,
        vec![image("https://img/1.png"), image("https://img/2.png")]
    );
}

#[rstest]
#[case::an_empty_object(b"{}")]
#[case::no_output(b"{\"usage\":{}}")]
#[case::an_empty_output(b"{\"output\":{}}")]
#[case::no_choices(b"{\"output\":{\"usage\":{}}}")]
#[case::an_empty_choices_list(b"{\"output\":{\"choices\":[]}}")]
#[case::a_choice_without_a_message(b"{\"output\":{\"choices\":[{}]}}")]
#[case::a_message_without_content(b"{\"output\":{\"choices\":[{\"message\":{}}]}}")]
#[case::an_empty_content_list(b"{\"output\":{\"choices\":[{\"message\":{\"content\":[]}}]}}")]
#[case::a_null_image(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"image\":null}]}}]}}"
)]
#[case::an_empty_image(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"image\":\"\"}]}}]}}"
)]
#[case::an_item_without_an_image(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"text\":\"hi\"}]}}]}}"
)]
fn the_reply_without_an_image_is_an_empty_list(#[case] body: &[u8]) {
    let data = decode_dashscope_image_reply(body, 200, &Headers::new()).unwrap();
    assert!(data.is_empty());
}

#[test]
fn the_reply_ignores_a_code_key_when_an_output_exists() {
    let body = br#"{"code":"InvalidParameter","output":{"choices":[{"message":{"content":[{"image":"https://img/3.png"}]}}]}}"#;
    let data = decode_dashscope_image_reply(body, 200, &Headers::new()).unwrap();
    assert_eq!(data, vec![image("https://img/3.png")]);
}

#[test]
fn a_code_without_an_output_is_an_error_carrying_its_message() {
    let body = br#"{"code":"InvalidParameter","message":"Size not supported","request_id":"r-1"}"#;
    let headers = headers(&[("x-request-id", "req-10")]);
    let error = reply_failure(decode_dashscope_image_reply(body, 200, &headers));
    assert_eq!(error.status_code, 200);
    assert_eq!(error.message, "Size not supported");
    assert_eq!(
        error.headers.get("x-request-id").map(String::as_str),
        Some("req-10")
    );
}

#[test]
fn a_code_without_a_message_carries_the_reply_text() {
    let body = br#"{"code":"InvalidParameter","request_id":"r-1"}"#;
    let error = reply_failure(decode_dashscope_image_reply(body, 200, &Headers::new()));
    assert_eq!(error.status_code, 200);
    assert!(error.message.contains("InvalidParameter"));
    assert!(error.message.contains("r-1"));
}

#[rstest]
#[case::a_null_message(br#"{"code":"x","message":null}"#)]
#[case::a_numeric_message(br#"{"code":"x","message":7}"#)]
fn a_code_with_a_non_text_message_carries_the_reply_text(#[case] body: &[u8]) {
    let error = reply_failure(decode_dashscope_image_reply(body, 200, &Headers::new()));
    assert_eq!(error.status_code, 200);
    assert!(error.message.starts_with('{'));
}

#[test]
fn a_non_200_reply_carries_the_status_the_body_and_the_headers() {
    let headers = headers(&[("x-request-id", "req-7")]);
    let error = reply_failure(decode_dashscope_image_reply(b"bad gateway", 503, &headers));
    assert_eq!(error.status_code, 503);
    assert_eq!(error.message, "bad gateway");
    assert_eq!(
        error.headers.get("x-request-id").map(String::as_str),
        Some("req-7")
    );
}

#[test]
fn a_non_200_reply_whose_body_is_not_utf8_still_carries_the_status() {
    let error = reply_failure(decode_dashscope_image_reply(
        &[0xff, 0xfe],
        500,
        &Headers::new(),
    ));
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("not UTF-8"));
}

#[test]
fn a_status_other_than_200_is_an_error() {
    let body = br#"{"output":{"choices":[]}}"#;
    let error = reply_failure(decode_dashscope_image_reply(body, 201, &Headers::new()));
    assert_eq!(error.status_code, 201);
    assert_eq!(error.message.as_bytes(), body);
}

#[test]
fn a_non_json_reply_carries_the_parse_failure_and_the_headers() {
    let headers = headers(&[("x-request-id", "req-8")]);
    let error = reply_failure(decode_dashscope_image_reply(b"not json", 200, &headers));
    assert_eq!(error.status_code, 200);
    assert!(error.message.starts_with("Failed to parse"));
    assert_eq!(
        error.headers.get("x-request-id").map(String::as_str),
        Some("req-8")
    );
}

#[rstest]
#[case::a_top_level_list(b"[]")]
#[case::a_top_level_string(b"\"text\"")]
#[case::a_top_level_null(b"null")]
#[case::an_output_that_is_null(b"{\"output\":null}")]
#[case::an_output_that_is_text(b"{\"output\":\"x\"}")]
#[case::choices_that_are_null(b"{\"output\":{\"choices\":null}}")]
#[case::choices_that_are_an_object(b"{\"output\":{\"choices\":{}}}")]
#[case::a_choice_that_is_text(b"{\"output\":{\"choices\":[\"x\"]}}")]
#[case::a_message_that_is_null(b"{\"output\":{\"choices\":[{\"message\":null}]}}")]
#[case::a_message_that_is_text(b"{\"output\":{\"choices\":[{\"message\":\"x\"}]}}")]
#[case::content_that_is_null(b"{\"output\":{\"choices\":[{\"message\":{\"content\":null}}]}}")]
#[case::content_that_is_text(b"{\"output\":{\"choices\":[{\"message\":{\"content\":\"x\"}}]}}")]
#[case::content_that_is_an_object(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":{\"a\":1}}}]}}"
)]
#[case::an_item_that_is_text(b"{\"output\":{\"choices\":[{\"message\":{\"content\":[\"x\"]}}]}}")]
#[case::an_image_that_is_a_number(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"image\":5}]}}]}}"
)]
#[case::an_image_that_is_a_bool(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"image\":true}]}}]}}"
)]
#[case::an_image_that_is_a_list(
    b"{\"output\":{\"choices\":[{\"message\":{\"content\":[{\"image\":[\"u\"]}]}}]}}"
)]
fn the_reply_rejects_a_shape_the_records_cannot_hold(#[case] body: &[u8]) {
    let headers = headers(&[("x-request-id", "req-9")]);
    let error = reply_failure(decode_dashscope_image_reply(body, 200, &headers));
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains("wrong type"));
    assert_eq!(
        error.headers.get("x-request-id").map(String::as_str),
        Some("req-9")
    );
}

#[rstest]
fn replays_the_dashscope_image_reply_corpus(
    #[files("fuzz/corpus/dashscope_image_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("read corpus file");
    let _decoded = decode_dashscope_image_reply(&bytes, 200, &Headers::new());
    let _failed = decode_dashscope_image_reply(&bytes, 503, &Headers::new());
}
