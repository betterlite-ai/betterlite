#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{
    ImageData, ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use super::error::StabilityGenerationError;
use super::reply::{StabilityReplyError, decode_stability_image};
use super::types::DEFAULT_GENERATION_ENDPOINT;
use super::{
    StabilityImageGenerationConfig, get_stability_image_generation_config, model_endpoint,
};

fn lookup(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(entries: &[(&str, &str)]) -> StabilityImageGenerationConfig {
    get_stability_image_generation_config("stability/sd3", &lookup(entries))
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

fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn upstream(status: u16, body: &str) -> ImageHttpResponse {
    ImageHttpResponse {
        status_code: status,
        headers: ImageHeaders::from([("x-request-id".to_owned(), "req".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

fn image(value: &str) -> ImageData {
    ImageData {
        b64_json: Some(value.to_owned()),
        url: None,
        revised_prompt: None,
    }
}

#[test]
fn the_supported_openai_params_are_n_size_and_response_format() {
    assert_eq!(
        StabilityImageGenerationConfig::SUPPORTED_OPENAI_PARAMS,
        ["n", "size", "response_format"]
    );
}

#[test]
fn the_adapter_reports_multipart_form_data() {
    assert!(StabilityImageGenerationConfig::MULTIPART_FORM_DATA);
}

#[rstest]
#[case::unknown_model("stability/not-a-model", DEFAULT_GENERATION_ENDPOINT)]
#[case::ultra("stability/stable-image-ultra", "/v2beta/stable-image/generate/ultra")]
#[case::core("stable-image-core", "/v2beta/stable-image/generate/core")]
#[case::uppercase("STABILITY/Stable-Image-Core", "/v2beta/stable-image/generate/core")]
#[case::sd3_family("sd3.5-large-turbo", DEFAULT_GENERATION_ENDPOINT)]
#[case::substring("my-stable-image-ultra-clone", "/v2beta/stable-image/generate/ultra")]
#[case::embedded_provider_prefix(
    "models/stability/stable-image-core",
    "/v2beta/stable-image/generate/core"
)]
fn the_endpoint_is_the_first_table_key_inside_the_lowercased_model(
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(model_endpoint(model), expected);
}

#[rstest]
#[case::default_base(&[], None, "https://api.stability.ai")]
#[case::environment_base(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    None,
    "https://env.test/v1"
)]
#[case::explicit_base_wins(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    Some("https://caller.test/base"),
    "https://caller.test/base"
)]
#[case::empty_explicit_base_falls_through(
    &[("STABILITY_API_BASE", "https://env.test/v1")],
    Some(""),
    "https://env.test/v1"
)]
#[case::trailing_slashes_are_trimmed(&[], Some("https://caller.test///"), "https://caller.test")]
fn the_complete_url_takes_an_explicit_base_before_the_environment(
    #[case] environment: &[(&str, &str)],
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let config = config(environment);

    assert_eq!(
        config.get_complete_url(api_base, "stability/sd3"),
        format!("{expected}/v2beta/stable-image/generate/sd3")
    );
}

#[test]
fn validate_environment_fails_without_a_key() {
    let error = config(&[])
        .validate_environment(ImageHeaders::new(), None)
        .expect_err("a missing key must fail");

    assert_eq!(error, StabilityGenerationError::MissingApiKey);
}

#[test]
fn validate_environment_overwrites_the_auth_and_accept_headers() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let mut headers = ImageHeaders::new();
    headers.insert("Authorization".to_owned(), "Bearer stale".to_owned());
    headers.insert("x-extra".to_owned(), "kept".to_owned());

    let headers = config
        .validate_environment(headers, None)
        .expect("the environment key is set");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
    assert_eq!(
        headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(headers.get("x-extra").map(String::as_str), Some("kept"));
}

#[test]
fn an_explicit_key_beats_the_environment_key() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let explicit = SecretString::from("caller-key");

    let headers = config
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the explicit key is set");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer caller-key")
    );
}

#[test]
fn an_empty_explicit_key_falls_through_to_the_environment_key() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);
    let explicit = SecretString::from("");

    let headers = config
        .validate_environment(ImageHeaders::new(), Some(&explicit))
        .expect("the environment key is set");

    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer server-key")
    );
}

#[test]
fn debug_never_shows_the_key() {
    let config = config(&[("STABILITY_API_KEY", "server-key")]);

    let rendered = format!("{config:?}");

    assert!(!rendered.contains("server-key"));
}

#[test]
fn the_factory_loads_the_stability_environment() {
    let config = config(&[
        ("STABILITY_API_KEY", "server-key"),
        ("STABILITY_API_BASE", "https://env.test"),
    ]);

    assert_eq!(config.settings.api_base, "https://env.test");
    assert_eq!(
        config.get_complete_url(None, "stability/sd3"),
        "https://env.test/v2beta/stable-image/generate/sd3"
    );
}

#[test]
fn a_known_size_maps_to_its_aspect_ratio() {
    let non_default = params(&[("size", text("1024x1024"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "stability/sd3",
        false,
    )
    .expect("size is supported");

    assert_eq!(
        mapped.get("aspect_ratio"),
        Some(&Json::Text("1:1".to_owned()))
    );
    assert!(!mapped.contains_key("size"));
}

#[test]
fn an_unknown_size_survives_mapping_as_size() {
    let non_default = params(&[("size", text("999x999"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "stability/sd3",
        false,
    )
    .expect("size is supported");

    assert_eq!(mapped.get("size"), Some(&Json::Text("999x999".to_owned())));
    assert!(!mapped.contains_key("aspect_ratio"));
}

#[test]
fn an_unknown_size_never_reaches_the_upstream_request() {
    let optional = params(&[("size", text("999x999"))]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("the request is built");

    let value = serde_json::to_value(&request).expect("the request serializes");
    assert_eq!(
        value,
        serde_json::json!({"prompt": "a cat", "output_format": "png"})
    );
}

#[test]
fn n_and_response_format_are_stashed_internal() {
    let non_default = params(&[("n", integer(2)), ("response_format", text("b64_json"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "stability/sd3",
        false,
    )
    .expect("both names are supported");

    assert_eq!(mapped.get("_n"), Some(&integer(2)));
    assert_eq!(mapped.get("_response_format"), Some(&text("b64_json")));
    assert!(!mapped.contains_key("n"));
    assert!(!mapped.contains_key("response_format"));
}

#[test]
fn an_existing_destination_key_is_not_overwritten() {
    let non_default = params(&[("size", text("1024x1024"))]);
    let optional = params(&[("size", text("1792x1024"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        optional,
        "stability/sd3",
        false,
    )
    .expect("size is supported");

    assert_eq!(
        mapped.get("size"),
        Some(&Json::Text("1792x1024".to_owned()))
    );
    assert!(!mapped.contains_key("aspect_ratio"));
}

#[test]
fn an_unsupported_parameter_raises_without_drop_params() {
    let non_default = params(&[("quality", text("hd"))]);

    let error = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "stability/sd3",
        false,
    )
    .expect_err("quality is unsupported");

    assert_eq!(
        error,
        StabilityGenerationError::UnsupportedParameter {
            parameter: "quality".to_owned(),
            model: "stability/sd3".to_owned(),
        }
    );
}

#[test]
fn drop_params_drops_an_unsupported_parameter() {
    let non_default = params(&[("quality", text("hd"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        ImageParams::new(),
        "stability/sd3",
        true,
    )
    .expect("drop_params drops quality");

    assert!(mapped.is_empty());
}

#[test]
fn an_unsupported_parameter_already_in_the_destination_survives() {
    let non_default = params(&[("quality", text("hd"))]);
    let optional = params(&[("quality", text("standard"))]);

    let mapped = StabilityImageGenerationConfig::map_openai_params(
        &non_default,
        optional,
        "stability/sd3",
        false,
    )
    .expect("an existing destination key is left alone");

    assert_eq!(
        mapped.get("quality"),
        Some(&Json::Text("standard".to_owned()))
    );
}

#[test]
fn the_request_starts_with_the_prompt_and_png_output() {
    let request = StabilityImageGenerationConfig::transform_image_generation_request(
        "a cat",
        &ImageParams::new(),
    )
    .expect("the request is built");

    assert_eq!(request.output_format, Some(Some("png".to_owned())));
    let value = serde_json::to_value(&request).expect("the request serializes");
    assert_eq!(
        value,
        serde_json::json!({"prompt": "a cat", "output_format": "png"})
    );
}

#[test]
fn a_null_provider_option_goes_upstream_as_null() {
    let optional = params(&[("negative_prompt", Json::Null), ("seed", Json::Null)]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("null values are accepted");

    assert_eq!(request.negative_prompt, Some(None));
    assert_eq!(request.seed, Some(None));
    let value = serde_json::to_value(&request).expect("the request serializes");
    assert_eq!(
        value,
        serde_json::json!({
            "prompt": "a cat",
            "output_format": "png",
            "negative_prompt": null,
            "seed": null
        })
    );
}

#[test]
fn internal_and_unknown_provider_keys_never_go_upstream() {
    let optional = params(&[
        ("_n", integer(2)),
        ("_response_format", text("b64_json")),
        ("model", text("sd3.5-large")),
        ("seed", integer(7)),
    ]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("the request is built");

    let value = serde_json::to_value(&request).expect("the request serializes");
    assert_eq!(
        value,
        serde_json::json!({
            "prompt": "a cat",
            "output_format": "png",
            "model": "sd3.5-large",
            "seed": 7
        })
    );
}

#[test]
fn the_generation_transform_never_forwards_an_image() {
    let optional = params(&[("image", text("aGVsbG8="))]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("the request is built");

    assert_eq!(request.image, None);
    let value = serde_json::to_value(&request).expect("the request serializes");
    assert_eq!(value.get("image"), None);
}

#[test]
fn a_supplied_output_format_replaces_png() {
    let optional = params(&[("output_format", text("webp"))]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("the request is built");

    assert_eq!(request.output_format, Some(Some("webp".to_owned())));
}

#[test]
fn a_null_output_format_goes_upstream_as_null() {
    let optional = params(&[("output_format", Json::Null)]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("a null output_format is accepted");

    assert_eq!(request.output_format, Some(None));
}

#[test]
fn a_wrong_typed_provider_option_names_its_field() {
    let optional = params(&[("seed", text("seven"))]);

    let error =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect_err("seed must be a number");

    assert_eq!(
        error,
        StabilityGenerationError::RequestField { field: "seed" }
    );
}

#[test]
fn strength_keeps_its_number_value() {
    let strength = Number::from_f64(0.5).expect("0.5 is a JSON number");
    let optional = params(&[("strength", Json::Number(strength.clone()))]);

    let request =
        StabilityImageGenerationConfig::transform_image_generation_request("a cat", &optional)
            .expect("the request is built");

    assert_eq!(request.strength, Some(Some(strength)));
}

#[test]
fn a_reply_image_is_appended_as_base64_data() {
    let mut response = ImageResponse::default();

    decode_stability_image(
        &upstream(200, r#"{"image":"abc","finish_reason":"SUCCESS","seed":9}"#),
        &mut response,
    )
    .expect("the reply holds an image");

    assert_eq!(response.created, 0);
    assert_eq!(response.data, Some(vec![image("abc")]));
}

#[test]
fn an_existing_response_keeps_its_created_value_and_data() {
    let mut response = ImageResponse {
        created: 42,
        data: Some(vec![image("old")]),
        usage: None,
    };

    decode_stability_image(&upstream(200, r#"{"image":"new"}"#), &mut response)
        .expect("the reply holds an image");

    assert_eq!(response.created, 42);
    assert_eq!(response.data, Some(vec![image("old"), image("new")]));
}

#[test]
fn a_missing_image_leaves_the_data_empty() {
    let mut response = ImageResponse::default();

    decode_stability_image(
        &upstream(200, r#"{"finish_reason":"SUCCESS"}"#),
        &mut response,
    )
    .expect("a reply without an image is not a failure");

    assert_eq!(response.data, Some(Vec::new()));
}

#[test]
fn an_empty_image_leaves_the_data_empty() {
    let mut response = ImageResponse::default();

    decode_stability_image(&upstream(200, r#"{"image":""}"#), &mut response)
        .expect("an empty image is not a failure");

    assert_eq!(response.data, Some(Vec::new()));
}

#[test]
fn a_non_filtered_finish_reason_is_not_an_error() {
    let mut response = ImageResponse::default();

    decode_stability_image(
        &upstream(200, r#"{"image":"abc","finish_reason":"ERROR"}"#),
        &mut response,
    )
    .expect("only CONTENT_FILTERED is an error");

    assert_eq!(response.data, Some(vec![image("abc")]));
}

#[test]
fn a_filtered_reply_fails_with_400_even_on_a_successful_status() {
    let error = decode_stability_image(
        &upstream(200, r#"{"image":"abc","finish_reason":"CONTENT_FILTERED"}"#),
        &mut ImageResponse::default(),
    )
    .expect_err("the content was filtered");

    assert_eq!(error.status(), 400);
    assert_eq!(
        error.headers().get("x-request-id").map(String::as_str),
        Some("req")
    );
    assert!(matches!(error, StabilityReplyError::ContentFiltered { .. }));
}

#[test]
fn a_present_errors_key_fails_with_the_upstream_status() {
    let error = decode_stability_image(
        &upstream(503, r#"{"errors":{"message":"boom"}}"#),
        &mut ImageResponse::default(),
    )
    .expect_err("the reply reports an error");

    assert_eq!(error.status(), 503);
    assert!(matches!(error, StabilityReplyError::Upstream { .. }));
}

#[test]
fn an_errors_key_with_a_null_value_still_fails() {
    let error = decode_stability_image(
        &upstream(200, r#"{"errors":null}"#),
        &mut ImageResponse::default(),
    )
    .expect_err("the key is present");

    assert_eq!(error.status(), 200);
}

#[test]
fn an_unparsable_reply_keeps_the_upstream_status() {
    let error = decode_stability_image(&upstream(502, "not json"), &mut ImageResponse::default())
        .expect_err("the body is not JSON");

    assert_eq!(error.status(), 502);
    assert!(matches!(error, StabilityReplyError::Malformed { .. }));
}

#[test]
fn a_non_object_reply_keeps_the_upstream_status() {
    let error = decode_stability_image(&upstream(201, "[]"), &mut ImageResponse::default())
        .expect_err("the body is not an object");

    assert_eq!(error.status(), 201);
    assert!(matches!(error, StabilityReplyError::NotObject { .. }));
}

#[test]
fn a_wrong_typed_image_names_the_field_and_keeps_the_status() {
    let error = decode_stability_image(
        &upstream(200, r#"{"image":5}"#),
        &mut ImageResponse::default(),
    )
    .expect_err("an image must be a string");

    assert_eq!(error.status(), 200);
    assert!(error.to_string().contains("does not fit"));
}

#[rstest]
fn replays_the_stability_image_corpus(#[files("fuzz/corpus/stability_image/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus seed is read");
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body,
    };

    drop(decode_stability_image(
        &response,
        &mut ImageResponse::default(),
    ));
}
