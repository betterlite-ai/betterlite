#![cfg(test)]

use std::io::Cursor;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::api::image_contracts::records::{ImageInputTokens, ImageParams, ImageUsage};
use crate::openrouter_image::edit::{
    MULTIPART_FORM_DATA, SourceImage, complete_url as edit_url,
    get_openrouter_image_edit_config as edit_config, request_body as edit_request_body,
    transform_response as edit_response, validate_environment as edit_headers,
};
use crate::openrouter_image::shared::{ImageConfigError, ImageHiddenParams};

use super::{
    COST_HEADER, CREATED, FailingReader, GIF, HEIC, JPEG, PNG, WEBP, b64, content_parts, headers,
    image_part, json, number, ok, params, raw, text,
};

#[test]
fn edit_requests_use_json_not_multipart() {
    assert!(!MULTIPART_FORM_DATA);
}

#[test]
fn edit_params_build_a_new_mapping_from_supported_keys() {
    let supplied = params(r#"{"size": "1024x1536", "n": 2}"#);
    let mapped = edit_config("m")
        .map_params(&supplied)
        .expect("the params map");
    assert_eq!(
        mapped,
        params(r#"{"image_config": {"aspect_ratio": "2:3"}, "n": 2}"#)
    );
    assert_eq!(supplied, params(r#"{"size": "1024x1536", "n": 2}"#));
}

#[test]
fn edit_params_put_image_config_where_the_first_mapped_key_was() {
    let mapped = edit_config("m")
        .map_params(&params(r#"{"n": 1, "size": "1024x1024"}"#))
        .expect("the params map");
    let order: Vec<&str> = mapped.keys().map(String::as_str).collect();
    assert_eq!(order, ["n", "image_config"]);
}

#[test]
fn edit_params_share_one_image_config_for_size_and_quality() {
    let mapped = edit_config("m")
        .map_params(&params(r#"{"quality": "low", "size": "1792x1024"}"#))
        .expect("the params map");
    assert_eq!(
        mapped,
        params(r#"{"image_config": {"aspect_ratio": "16:9", "image_size": "1K"}}"#)
    );
    let Some(Json::Object(config)) = mapped.get("image_config") else {
        panic!("image_config is an object");
    };
    let order: Vec<&str> = config.keys().map(String::as_str).collect();
    assert_eq!(order, ["image_size", "aspect_ratio"]);
}

#[test]
fn edit_params_drop_unsupported_keys() {
    let mapped = edit_config("m")
        .map_params(&params(
            r#"{"model": "x", "response_format": "b64_json", "size": "1024x1024", "style": "vivid"}"#,
        ))
        .expect("the params map");
    assert_eq!(
        mapped,
        params(r#"{"image_config": {"aspect_ratio": "1:1"}}"#)
    );
}

#[test]
fn edit_params_add_no_image_config_for_an_unknown_quality() {
    let mapped = edit_config("m")
        .map_params(&params(r#"{"quality": "ultra", "n": 1}"#))
        .expect("the params map");
    assert_eq!(mapped, params(r#"{"n": 1}"#));
}

#[rstest]
#[case::number(r#"{"size": 1024}"#)]
#[case::null(r#"{"size": null}"#)]
fn edit_params_reject_a_size_that_is_not_a_string(#[case] supplied: &str) {
    let error = edit_config("m")
        .map_params(&params(supplied))
        .expect_err("size must be a string");
    assert!(matches!(error, ImageConfigError::Size));
}

#[rstest]
#[case::number(r#"{"quality": 1}"#)]
#[case::list(r#"{"quality": []}"#)]
fn edit_params_reject_a_quality_that_is_not_a_string(#[case] supplied: &str) {
    let error = edit_config("m")
        .map_params(&params(supplied))
        .expect_err("quality must be a string");
    assert!(matches!(error, ImageConfigError::Quality));
}

#[rstest]
#[case::default(None, None, "https://openrouter.ai/api/v1/chat/completions")]
#[case::environment(
    None,
    Some("http://env.test/v1"),
    "http://env.test/v1/chat/completions"
)]
#[case::api_base(
    Some("http://api.test/v1"),
    Some("http://env.test/v1"),
    "http://api.test/v1/chat/completions"
)]
#[case::empty(Some(""), Some(""), "https://openrouter.ai/api/v1/chat/completions")]
#[case::trailing_slashes(
    Some("http://api.test/v1//"),
    None,
    "http://api.test/v1/chat/completions"
)]
#[case::chat_endpoint(
    Some("http://api.test/chat/completions/"),
    None,
    "http://api.test/chat/completions"
)]
fn edit_urls_take_the_api_base_then_the_environment(
    #[case] api_base: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(edit_url(api_base, environment), expected);
}

#[test]
fn edit_headers_require_a_key() {
    let error =
        edit_headers(headers(&[("X-Extra", "1")]), None, None, None).expect_err("no key is set");
    assert!(matches!(error, ImageConfigError::MissingApiKey));
    assert_eq!(error.to_string(), "OPENROUTER_API_KEY is not set");
    let empty = SecretString::from("");
    let error = edit_headers(headers(&[]), Some(&empty), Some(&empty), Some(&empty))
        .expect_err("an empty key counts as unset");
    assert!(matches!(error, ImageConfigError::MissingApiKey));
}

#[test]
fn edit_headers_take_the_first_non_empty_key() {
    let api = SecretString::from("api-key");
    let global = SecretString::from("global-key");
    let environment = SecretString::from("environment-key");
    let chosen = edit_headers(
        headers(&[("X-Extra", "1")]),
        Some(&api),
        Some(&global),
        Some(&environment),
    )
    .expect("a key is set");
    assert_eq!(
        chosen,
        headers(&[("X-Extra", "1"), ("Authorization", "Bearer api-key")])
    );
    let chosen =
        edit_headers(headers(&[]), None, Some(&global), Some(&environment)).expect("a key is set");
    assert_eq!(chosen["Authorization"], "Bearer global-key");
    let chosen = edit_headers(headers(&[]), None, None, Some(&environment)).expect("a key is set");
    assert_eq!(chosen["Authorization"], "Bearer environment-key");
}

#[test]
fn edit_requests_carry_the_image_and_the_prompt() {
    let optional = params(
        r#"{"n": 2, "image_config": {"aspect_ratio": "1:1"}, "model": "ignored", "modalities": ["ignored"]}"#,
    );
    let mut images = [Some(SourceImage::Bytes(PNG))];
    let (body, files) = edit_request_body(
        "openrouter/model",
        Some("Edit this"),
        &mut images,
        &optional,
    )
    .expect("the request builds");
    assert!(files.is_empty());
    assert_eq!(body["model"], text("openrouter/model"));
    assert_eq!(body["modalities"], json(r#"["image", "text"]"#));
    assert_eq!(body["n"], number(2));
    assert_eq!(body["image_config"], json(r#"{"aspect_ratio": "1:1"}"#));
    let parts = content_parts(&body);
    assert_eq!(
        parts,
        vec![
            image_part("image/png", PNG),
            json(r#"{"type": "text", "text": "Edit this"}"#),
        ]
    );
}

#[test]
fn edit_requests_skip_a_null_image_and_an_empty_prompt() {
    let mut images = [None, Some(SourceImage::Bytes(JPEG))];
    let (body, _files) = edit_request_body("m", Some(""), &mut images, &ImageParams::new())
        .expect("the request builds");
    assert_eq!(content_parts(&body), vec![image_part("image/jpeg", JPEG)]);
}

#[test]
fn edit_requests_without_images_or_prompt_have_empty_content() {
    let mut images: [Option<SourceImage<'_>>; 0] = [];
    let (body, _files) =
        edit_request_body("m", None, &mut images, &ImageParams::new()).expect("the request builds");
    assert!(content_parts(&body).is_empty());
}

#[test]
fn edit_requests_read_a_reader_from_the_start_and_restore_its_position() {
    let mut cursor = Cursor::new(Vec::from(PNG));
    cursor.set_position(4);
    let mut images = [Some(SourceImage::Reader(&mut cursor))];
    let (body, _files) =
        edit_request_body("m", None, &mut images, &ImageParams::new()).expect("the request builds");
    assert_eq!(cursor.position(), 4);
    assert_eq!(content_parts(&body), vec![image_part("image/png", PNG)]);
}

#[rstest]
#[case::png(PNG, "image/png")]
#[case::jpeg(JPEG, "image/jpeg")]
#[case::gif(GIF, "image/gif")]
#[case::webp(WEBP, "image/webp")]
#[case::heic(HEIC, "image/heic")]
#[case::unreadable(b"not an image", "image/png")]
#[case::empty(b"", "image/png")]
fn edit_requests_detect_the_image_mime_type(#[case] bytes: &[u8], #[case] mime: &str) {
    let mut images = [Some(SourceImage::Bytes(bytes))];
    let (body, _files) =
        edit_request_body("m", None, &mut images, &ImageParams::new()).expect("the request builds");
    assert_eq!(content_parts(&body), vec![image_part(mime, bytes)]);
}

#[test]
fn edit_requests_reject_an_unsupported_image() {
    let mut images = [Some(SourceImage::Unsupported)];
    let error = edit_request_body("m", None, &mut images, &ImageParams::new())
        .expect_err("the image is unsupported");
    assert!(matches!(error, ImageConfigError::UnsupportedImage));
    assert_eq!(
        error.to_string(),
        "Unsupported image type for OpenRouter image edit."
    );
}

#[test]
fn edit_requests_report_a_reader_that_cannot_be_read() {
    let mut reader = FailingReader;
    let mut images = [Some(SourceImage::Reader(&mut reader))];
    let error = edit_request_body("m", None, &mut images, &ImageParams::new())
        .expect_err("the reader fails");
    assert!(matches!(error, ImageConfigError::ImageRead { .. }));
}

#[test]
fn edit_responses_start_from_a_new_image_response() {
    let raw = ok(
        r#"{"model": "reply/model", "choices": [{"message": {"images": [{"image_url": {"url": "data:image/png;base64,CCCC"}}]}}]}"#,
    );
    let (response, hidden) =
        edit_response(&raw, CREATED, "requested/model").expect("the reply transforms");
    assert_eq!(response.created, CREATED);
    assert_eq!(response.data, Some(vec![b64("CCCC")]));
    assert_eq!(response.usage, Some(ImageUsage::default()));
    assert_eq!(
        hidden,
        ImageHiddenParams {
            additional_headers: IndexMap::new(),
            response_cost_details: None,
            model: Some(text("reply/model")),
        }
    );
}

#[test]
fn edit_responses_split_prompt_tokens_into_input_image_tokens() {
    let raw = ok(
        r#"{"usage": {"prompt_tokens": 10, "total_tokens": 13, "completion_tokens_details": {"image_tokens": 3}, "prompt_tokens_details": {"image_tokens": 4}}}"#,
    );
    let (response, _hidden) = edit_response(&raw, CREATED, "m").expect("the reply transforms");
    assert_eq!(
        response.usage,
        Some(ImageUsage {
            input_tokens: 10,
            input_tokens_details: ImageInputTokens {
                image_tokens: 4,
                text_tokens: 6,
            },
            output_tokens: 3,
            total_tokens: 13,
        })
    );
}

#[test]
fn edit_responses_read_absent_prompt_details_as_zero() {
    let bodies = [
        r#"{"usage": {"prompt_tokens": 10}}"#,
        r#"{"usage": {"prompt_tokens": 10, "prompt_tokens_details": {}}}"#,
        r#"{"usage": {"prompt_tokens": 10, "prompt_tokens_details": null}}"#,
    ];
    let usages: Vec<Option<ImageUsage>> = bodies
        .iter()
        .map(|body| {
            edit_response(&ok(body), CREATED, "m")
                .expect("the reply transforms")
                .0
                .usage
        })
        .collect();
    let text_tokens: Vec<i64> = usages
        .iter()
        .flatten()
        .map(|usage| usage.input_tokens_details.text_tokens)
        .collect();
    assert_eq!(text_tokens, vec![10, 10, 10]);
}

#[test]
fn edit_responses_let_text_tokens_go_negative() {
    let raw =
        ok(r#"{"usage": {"prompt_tokens": 2, "prompt_tokens_details": {"image_tokens": 5}}}"#);
    let (response, _hidden) = edit_response(&raw, CREATED, "m").expect("the reply transforms");
    let Some(usage) = response.usage else {
        panic!("the usage is set");
    };
    assert_eq!(usage.input_tokens_details.image_tokens, 5);
    assert_eq!(usage.input_tokens_details.text_tokens, -3);
}

#[test]
fn edit_responses_reject_text_tokens_that_do_not_fit() {
    let raw = ok(
        r#"{"usage": {"prompt_tokens": -9223372036854775808, "prompt_tokens_details": {"image_tokens": 1}}}"#,
    );
    let error = edit_response(&raw, CREATED, "m").expect_err("the difference does not fit");
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "Error transforming OpenRouter image edit response: usage.prompt_tokens minus its input image tokens does not fit in a whole number"
    );
}

#[test]
fn edit_responses_store_the_cost_and_merge_details() {
    let raw = ok(r#"{"usage": {"cost": 0.5, "cost_details": {"upstream": 0.4}}}"#);
    let (_, hidden) = edit_response(&raw, CREATED, "m").expect("the reply transforms");
    let expected_details = IndexMap::from([(String::from("upstream"), json("0.4"))]);
    assert_eq!(hidden.response_cost_details, Some(expected_details));
    let Some(Json::Number(cost)) = hidden.additional_headers.get(COST_HEADER) else {
        panic!("the cost header is set");
    };
    assert_eq!(cost.as_f64(), Some(0.5_f64));
}

#[test]
fn edit_responses_ignore_an_empty_usage() {
    let (response, hidden) =
        edit_response(&ok(r#"{"usage": {}}"#), CREATED, "m").expect("the reply transforms");
    assert_eq!(response.usage, Some(ImageUsage::default()));
    assert!(hidden.additional_headers.is_empty());
    assert_eq!(hidden.model, Some(text("m")));
}

#[rstest]
#[case::prompt_text(
    r#"{"prompt_tokens": "10"}"#,
    "usage.prompt_tokens must be a whole number"
)]
#[case::prompt_float(
    r#"{"prompt_tokens": 1.5}"#,
    "usage.prompt_tokens must be a whole number"
)]
#[case::total_float(
    r#"{"total_tokens": 1.5}"#,
    "usage.total_tokens must be a whole number"
)]
#[case::completion_details_list(
    r#"{"completion_tokens_details": []}"#,
    "usage.completion_tokens_details must be an object"
)]
#[case::completion_image_bool(
    r#"{"completion_tokens_details": {"image_tokens": true}}"#,
    "usage.completion_tokens_details.image_tokens must be a whole number"
)]
#[case::prompt_details_list(
    r#"{"prompt_tokens_details": []}"#,
    "usage.prompt_tokens_details must be an object"
)]
#[case::prompt_image_text(
    r#"{"prompt_tokens_details": {"image_tokens": "4"}}"#,
    "usage.prompt_tokens_details.image_tokens must be a whole number"
)]
#[case::cost_text(r#"{"cost": "0.5"}"#, "usage.cost must be a finite number")]
#[case::cost_details_text(r#"{"cost_details": "x"}"#, "usage.cost_details must be an object")]
fn edit_responses_reject_a_usage_field_of_the_wrong_type(
    #[case] usage: &str,
    #[case] reason: &str,
) {
    let body = format!(r#"{{"usage": {usage}}}"#);
    let error = edit_response(&ok(&body), CREATED, "m").expect_err("the usage field is wrong");
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert_eq!(
        error.message,
        format!("Error transforming OpenRouter image edit response: {reason}")
    );
}

#[test]
fn edit_parse_failures_keep_the_raw_status_and_headers() {
    let raw = raw(502, headers(&[("X-Raw", "1")]), "<html>");
    let error = edit_response(&raw, CREATED, "m").expect_err("the body is not JSON");
    assert_eq!(error.status_code, 502);
    assert_eq!(error.headers, headers(&[("X-Raw", "1")]));
    assert!(
        error
            .message
            .starts_with("Error parsing OpenRouter response: ")
    );
}

#[test]
fn edit_shape_failures_are_a_500_with_no_headers() {
    let raw = raw(200, headers(&[]), r#"{"choices": {}}"#);
    let error = edit_response(&raw, CREATED, "m").expect_err("choices must be a list");
    assert_eq!(error.status_code, 500);
    assert!(error.headers.is_empty());
    assert_eq!(
        error.message,
        "Error transforming OpenRouter image edit response: choices must be a list"
    );
}
