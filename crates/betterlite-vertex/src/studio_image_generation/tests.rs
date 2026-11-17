#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::usage::{DetailMap, ImageUsageDetails};
use betterlite_models::price_lookup::PriceRecord;
use betterlite_openai::api::image_contracts::records::ImageInputTokens;
use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

use super::cost::{ImageCostResponse, generation_cost};
use super::decode::decode_studio_image;
use super::environment::validate_environment;
use super::error::StudioImageError;
use super::factory::{GOOGLE_IMAGE_GEN_CONFIG, get_gemini_image_generation_config};
use super::options::{map_openai_params, supported_openai_params};
use super::request::transform_image_generation_request;
use super::response::{ImageGenerationResponse, transform_image_generation_response};
use super::settings::{DEFAULT_API_BASE, Settings};
use super::url::complete_url;
use super::usage::{ImageGenerationUsage, decode_image_usage, transform_gemini_image_usage};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;
use crate::studio_identity::Headers;
use crate::vertex_image_policy::cost::ImagePricing;

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("object source")
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn reply(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn studio_settings(pairs: &[(&str, &str)]) -> Settings {
    Settings::from_environment(&environment(pairs))
}

fn key(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn header<'headers>(headers: &'headers Headers, name: &str) -> Option<&'headers str> {
    headers.get(name).map(|value| value.expose_secret())
}

fn usage(source: &str) -> ImageGenerationUsage {
    let raw: Box<RawValue> = serde_json::from_str(source).expect("usage source");
    let metadata = decode_image_usage(&raw).expect("the usage metadata decodes");
    transform_gemini_image_usage(&metadata).expect("the usage transforms")
}

fn generated(model: &str, body: &str) -> ImageGenerationResponse {
    transform_image_generation_response(model, &reply(body)).expect("the reply transforms")
}

fn encoded_usage(usage: &ImageGenerationUsage) -> Json {
    json(&serde_json::to_string(usage).expect("the usage encodes"))
}

const GEMINI: &str = "gemini-2.5-flash-image";
const IMAGEN: &str = "imagen-3.0-generate-002";

// prompt text 1 + image 3, output image 6 only: the shape the priced tests feed the calculator
const CANONICAL_USAGE: &str = r#"{"promptTokenCount":4,"candidatesTokenCount":6,"totalTokenCount":10,"promptTokensDetails":[{"modality":"TEXT","tokenCount":1},{"modality":"IMAGE","tokenCount":3}],"candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":6}]}"#;

#[rstest]
#[case::gemini(
    "https://studio.test/v1beta/",
    GEMINI,
    "https://studio.test/v1beta/models/gemini-2.5-flash-image:generateContent"
)]
#[case::gemini_prefixed(
    "https://studio.test/v1beta",
    "gemini/gemini-2.5-flash-image",
    "https://studio.test/v1beta/models/gemini/gemini-2.5-flash-image:generateContent"
)]
#[case::imagen(
    "https://studio.test/v1beta",
    IMAGEN,
    "https://studio.test/v1beta/models/imagen-3.0-generate-002:predict"
)]
#[case::imagen_legacy(
    "https://studio.test",
    "imagegeneration@006",
    "https://studio.test/models/imagegeneration@006:predict"
)]
fn an_explicit_base_keeps_one_slash_and_picks_the_endpoint(
    #[case] base: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    let settings = studio_settings(&[]);
    assert_eq!(complete_url(model, Some(base), &settings), expected);
}

#[rstest]
#[case::absent(&[], DEFAULT_API_BASE)]
#[case::empty(&[("GEMINI_API_BASE", "")], DEFAULT_API_BASE)]
#[case::explicit(&[("GEMINI_API_BASE", "https://studio.test/v1beta")], "https://studio.test/v1beta")]
fn the_base_falls_back_to_the_studio_endpoint(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = studio_settings(pairs);
    assert_eq!(settings.api_base, expected);
    assert_eq!(
        complete_url(GEMINI, None, &settings),
        format!("{}/models/{}:generateContent", expected, GEMINI)
    );
}

#[rstest]
#[case::set(&[("GEMINI_API_KEY", "env-key")], Some("env-key"))]
#[case::empty(&[("GEMINI_API_KEY", "")], None)]
#[case::absent(&[], None)]
fn the_environment_key_is_read_only_when_it_is_not_empty(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = studio_settings(pairs);
    let found = settings
        .gemini_api_key
        .as_ref()
        .map(|value| value.expose_secret());
    assert_eq!(found, expected);
}

#[rstest]
#[case::gemini(GEMINI)]
#[case::imagen(IMAGEN)]
fn the_factory_answers_the_studio_class_for_any_model(#[case] model: &str) {
    let lookup = environment(&[("GEMINI_API_KEY", "env-key")]);
    let config = get_gemini_image_generation_config(model, &lookup);
    assert_eq!(GOOGLE_IMAGE_GEN_CONFIG, "GoogleImageGenConfig");
    assert_eq!(config.settings.api_base, DEFAULT_API_BASE);
    assert_eq!(
        config
            .settings
            .gemini_api_key
            .as_ref()
            .map(|value| value.expose_secret()),
        Some("env-key"),
    );
}

#[test]
fn an_explicit_key_overwrites_the_google_header_and_the_content_type() {
    let caller = environment(&[
        ("x-goog-api-key", "caller-key"),
        ("Content-Type", "text/plain"),
        ("x-extra", "kept"),
    ]);
    let headers = validate_environment(&caller, Some(&key("explicit")), &studio_settings(&[]))
        .expect("the key");
    assert_eq!(header(&headers, "x-goog-api-key"), Some("explicit"));
    assert_eq!(header(&headers, "Content-Type"), Some("application/json"));
    assert_eq!(header(&headers, "x-extra"), Some("kept"));
}

#[test]
fn the_environment_key_stands_in_when_the_caller_gives_none() {
    let settings = studio_settings(&[("GEMINI_API_KEY", "env-key")]);
    let headers = validate_environment(&environment(&[]), None, &settings).expect("the key");
    assert_eq!(header(&headers, "x-goog-api-key"), Some("env-key"));
    assert_eq!(headers.len(), 2);
}

#[test]
fn a_google_key_alone_is_not_enough() {
    let settings = studio_settings(&[("GOOGLE_API_KEY", "google-key")]);
    let error = validate_environment(&environment(&[]), None, &settings)
        .expect_err("the Studio adapter never reads GOOGLE_API_KEY");
    assert!(matches!(error, StudioImageError::MissingApiKey));
    assert_eq!(error.to_string(), "GEMINI_API_KEY is not set");
}

#[test]
fn an_empty_key_falls_through_to_the_environment() {
    let empty = key("");
    let settings = studio_settings(&[("GEMINI_API_KEY", "env-key")]);
    let headers =
        validate_environment(&environment(&[]), Some(&empty), &settings).expect("the key");
    assert_eq!(header(&headers, "x-goog-api-key"), Some("env-key"));
    let blank = Settings {
        gemini_api_key: Some(empty.clone()),
        api_base: DEFAULT_API_BASE.to_owned(),
    };
    let error = validate_environment(&environment(&[]), Some(&empty), &blank)
        .expect_err("two empty keys are still no key");
    assert!(matches!(error, StudioImageError::MissingApiKey));
}

#[rstest]
#[case::gemini(GEMINI, &["n", "size", "imageConfig", "tools", "web_search_options"])]
#[case::imagen(IMAGEN, &["n", "size"])]
#[case::empty("", &["n", "size"])]
fn the_supported_params_follow_the_model_family(#[case] model: &str, #[case] expected: &[&str]) {
    assert_eq!(supported_openai_params(model), expected);
}

#[rstest]
#[case::gemini(GEMINI)]
#[case::imagen(IMAGEN)]
fn a_string_image_config_is_not_parsed_for_studio(#[case] model: &str) {
    let params = object(r#"{"imageConfig":"{\"aspectRatio\":\"1:1\"}"}"#);
    let mapped = map_openai_params(&params, &object("{}"), model, None).expect("mapping");
    assert!(mapped.get("imageConfig").is_none());
}

#[test]
fn the_gemini_mapping_adds_search_tools_and_the_imagen_mapping_drops_them() {
    let params = object(r#"{"web_search_options":{"search_context_size":"low"}}"#);
    let gemini = map_openai_params(&params, &object("{}"), GEMINI, None).expect("mapping");
    assert_eq!(gemini, object(r#"{"tools":[{"googleSearch":{}}]}"#));
    let imagen = map_openai_params(&params, &object("{}"), IMAGEN, None).expect("mapping");
    assert_eq!(imagen, object("{}"));
}

#[rstest]
#[case::gemini(
    GEMINI,
    r#"{"n":2,"size":"1024x1024"}"#,
    r#"{"sampleCount":2,"imageConfig":{"aspectRatio":"1:1"}}"#
)]
#[case::imagen(
    IMAGEN,
    r#"{"n":2,"size":"1024x1024"}"#,
    r#"{"sampleCount":2,"aspectRatio":"1:1"}"#
)]
fn the_options_map_to_the_family_request(
    #[case] model: &str,
    #[case] params: &str,
    #[case] expected: &str,
) {
    let mapped = map_openai_params(&object(params), &object("{}"), model, None).expect("mapping");
    assert_eq!(mapped, object(expected));
}

#[test]
fn the_gemini_request_omits_the_role_and_asks_for_both_modalities() {
    let body = transform_image_generation_request(GEMINI, "a cat", &object("{}"), None)
        .expect("the request builds");
    assert_eq!(
        body,
        object(
            r#"{"contents":[{"parts":[{"text":"a cat"}]}],"generationConfig":{"response_modalities":["IMAGE","TEXT"]}}"#
        )
    );
}

#[rstest]
#[case::empty_tools(r#"{"tools":[]}"#, None)]
#[case::tools(
    r#"{"tools":[{"function_declarations":[{"name":"f"}]}]}"#,
    Some("tools")
)]
#[case::empty_config(r#"{"toolConfig":{}}"#, None)]
#[case::config(r#"{"toolConfig":{"a":1}}"#, Some("toolConfig"))]
#[case::zero_config(r#"{"toolConfig":0}"#, None)]
#[case::false_config(r#"{"toolConfig":false}"#, None)]
fn the_gemini_request_copies_only_truthy_tools(#[case] source: &str, #[case] copied: Option<&str>) {
    let params = object(source);
    let body = transform_image_generation_request(GEMINI, "a cat", &params, None)
        .expect("the request builds");
    for key in ["tools", "toolConfig"] {
        let expected = if Some(key) == copied {
            params.get(key)
        } else {
            None
        };
        assert_eq!(body.get(key), expected, "the {key} value");
    }
}

#[test]
fn the_gemini_request_passes_the_image_config_and_the_candidate_count() {
    let params = object(r#"{"n":2,"imageConfig":{"aspectRatio":"2:3"},"imageSize":"2K"}"#);
    let body = transform_image_generation_request(GEMINI, "a cat", &params, None)
        .expect("the request builds");
    assert_eq!(
        body.get("generationConfig"),
        Some(&json(
            r#"{"response_modalities":["IMAGE","TEXT"],"imageConfig":{"aspectRatio":"2:3"},"candidateCount":2}"#
        ))
    );
}

#[test]
fn the_imagen_request_keeps_its_typed_parameters() {
    let params = object(
        r#"{"sampleCount":2,"aspectRatio":"16:9","unknown":1,"background":null,"style":"","user":"u"}"#,
    );
    let body = transform_image_generation_request(IMAGEN, "a cat", &params, None)
        .expect("the request builds");
    assert_eq!(
        body,
        object(
            r#"{"instances":[{"prompt":"a cat"}],"parameters":{"sampleCount":2,"aspectRatio":"16:9","style":"","user":"u"}}"#
        )
    );
}

#[rstest]
#[case::count_text(r#"{"sampleCount":"2"}"#, "sampleCount", "an integer")]
#[case::count_float(r#"{"sampleCount":1.5}"#, "sampleCount", "an integer")]
#[case::ratio_number(r#"{"aspectRatio":5}"#, "aspectRatio", "a string")]
#[case::ratio_list(r#"{"aspectRatio":["1:1"]}"#, "aspectRatio", "a string")]
fn an_imagen_parameter_of_the_wrong_type_is_refused(
    #[case] params: &str,
    #[case] param: &'static str,
    #[case] expected: &'static str,
) {
    let error = transform_image_generation_request(IMAGEN, "a cat", &object(params), None)
        .expect_err("the parameter type is declared");
    assert!(matches!(
        error,
        StudioImageError::FieldType { param: found, expected: wanted } if found == param && wanted == expected
    ));
    assert_eq!(error.to_string(), format!("`{param}` must be {expected}"));
}

#[test]
fn the_usage_carries_the_prompt_and_completion_aliases() {
    let usage = usage(
        r#"{"promptTokenCount":4,"candidatesTokenCount":6,"totalTokenCount":10,"promptTokensDetails":[{"modality":"TEXT","tokenCount":4}]}"#,
    );
    assert_eq!(
        encoded_usage(&usage),
        json(
            r#"{"input_tokens":4,"input_tokens_details":{"image_tokens":0,"text_tokens":4},"output_tokens":6,"total_tokens":10,"prompt_tokens":4,"prompt_tokens_details":{"image_tokens":0,"text_tokens":4},"completion_tokens":6,"completion_tokens_details":{"image_tokens":6,"text_tokens":0},"output_tokens_details":{"image_tokens":6,"text_tokens":0}}"#
        )
    );
}

#[test]
fn text_and_image_details_sum_on_both_sides() {
    let usage = usage(
        r#"{"promptTokenCount":10,"candidatesTokenCount":7,"totalTokenCount":17,"promptTokensDetails":[{"modality":"TEXT","tokenCount":4},{"modality":"IMAGE","tokenCount":6}],"candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":5}]}"#,
    );
    let expected = ImageGenerationUsage {
        input_tokens: 10,
        input_tokens_details: ImageInputTokens {
            image_tokens: 6,
            text_tokens: 4,
        },
        output_tokens: 7,
        total_tokens: 17,
        prompt_tokens: 10,
        prompt_tokens_details: ImageInputTokens {
            image_tokens: 6,
            text_tokens: 4,
        },
        completion_tokens: 7,
        completion_tokens_details: ImageInputTokens {
            image_tokens: 5,
            text_tokens: 2,
        },
        output_tokens_details: ImageInputTokens {
            image_tokens: 5,
            text_tokens: 2,
        },
        web_search_requests: None,
    };
    assert_eq!(usage, expected);
}

#[test]
fn a_missing_candidate_breakdown_puts_the_output_tokens_on_images() {
    let usage = usage(
        r#"{"candidatesTokenCount":7,"promptTokensDetails":[{"modality":"TEXT","tokenCount":3}]}"#,
    );
    assert_eq!(usage.output_tokens_details.image_tokens, 7);
    assert_eq!(usage.output_tokens_details.text_tokens, 0);
    assert_eq!(usage.input_tokens_details.image_tokens, 0);
    assert_eq!(usage.input_tokens_details.text_tokens, 3);
}

#[test]
fn a_positive_remainder_goes_to_text_and_overreported_details_stay() {
    let remainder = usage(
        r#"{"candidatesTokenCount":9,"candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":4},{"modality":"TEXT","tokenCount":1}]}"#,
    );
    assert_eq!(remainder.output_tokens_details.image_tokens, 4);
    assert_eq!(remainder.output_tokens_details.text_tokens, 5);
    let overreported = usage(
        r#"{"candidatesTokenCount":2,"candidatesTokensDetails":[{"modality":"IMAGE","tokenCount":4}]}"#,
    );
    assert_eq!(overreported.output_tokens_details.image_tokens, 4);
    assert_eq!(overreported.output_tokens_details.text_tokens, 0);
}

#[test]
fn the_camel_case_breakdown_wins_over_the_snake_case_one() {
    let usage = usage(
        r#"{"promptTokenCount":5,"promptTokensDetails":[{"modality":"TEXT","tokenCount":5}],"prompt_tokens_details":[{"modality":"IMAGE","tokenCount":5}]}"#,
    );
    assert_eq!(usage.input_tokens_details.text_tokens, 5);
    assert_eq!(usage.input_tokens_details.image_tokens, 0);
}

#[test]
fn an_empty_camel_case_breakdown_keeps_the_snake_case_one_out() {
    let usage = usage(
        r#"{"candidatesTokenCount":4,"candidatesTokensDetails":[],"candidates_tokens_details":[{"modality":"TEXT","tokenCount":4}]}"#,
    );
    assert_eq!(usage.output_tokens_details.image_tokens, 4);
    assert_eq!(usage.output_tokens_details.text_tokens, 0);
}

#[test]
fn a_null_or_non_list_breakdown_falls_through_to_the_snake_case_one() {
    let null_breakdown = usage(
        r#"{"candidatesTokenCount":3,"candidatesTokensDetails":null,"candidates_tokens_details":[{"modality":"TEXT","tokenCount":1}]}"#,
    );
    assert_eq!(null_breakdown.output_tokens_details.text_tokens, 2);
    let scalar = usage(
        r#"{"candidatesTokenCount":3,"candidatesTokensDetails":"x","candidates_tokens_details":[{"modality":"IMAGE","tokenCount":3}]}"#,
    );
    assert_eq!(scalar.output_tokens_details.image_tokens, 3);
}

#[test]
fn a_lowercase_modality_still_counts() {
    let usage = usage(
        r#"{"promptTokensDetails":[{"modality":"text","tokenCount":2}],"candidatesTokenCount":3,"candidatesTokensDetails":[{"modality":"image","tokenCount":3}]}"#,
    );
    assert_eq!(usage.input_tokens_details.text_tokens, 2);
    assert_eq!(usage.output_tokens_details.image_tokens, 3);
}

#[test]
fn a_token_count_that_is_not_an_integer_counts_zero() {
    let usage = usage(
        r#"{"promptTokensDetails":[{"modality":"TEXT","tokenCount":2.5},{"modality":"TEXT","tokenCount":"3"},{"modality":"TEXT","tokenCount":4,"token_count":9}]}"#,
    );
    assert_eq!(usage.input_tokens_details.text_tokens, 4);
}

#[test]
fn the_snake_case_token_count_answers_when_the_camel_one_is_absent() {
    let usage = usage(r#"{"promptTokensDetails":[{"modality":"TEXT","token_count":4}]}"#);
    assert_eq!(usage.input_tokens_details.text_tokens, 4);
}

#[test]
fn a_detail_without_a_modality_or_with_another_one_counts_nothing() {
    let usage = usage(
        r#"{"promptTokensDetails":[{"tokenCount":4},{"modality":"AUDIO","tokenCount":5},"x"]}"#,
    );
    assert_eq!(usage.input_tokens_details, ImageInputTokens::default());
}

#[test]
fn token_counts_that_leave_the_signed_range_are_refused() {
    let prompt = format!(
        r#"{{"promptTokensDetails":[{{"modality":"TEXT","tokenCount":{}}},{{"modality":"TEXT","tokenCount":{}}}]}}"#,
        i64::MAX,
        i64::MAX
    );
    let error = transform_usage_error(&prompt);
    assert!(matches!(
        error,
        StudioImageError::TokenOverflow {
            field: "promptTokensDetails"
        }
    ));
    let candidates = format!(
        r#"{{"candidatesTokenCount":2,"candidatesTokensDetails":[{{"modality":"IMAGE","tokenCount":{}}},{{"modality":"IMAGE","tokenCount":{}}}]}}"#,
        i64::MAX,
        i64::MAX
    );
    let error = transform_usage_error(&candidates);
    assert!(matches!(
        error,
        StudioImageError::TokenOverflow {
            field: "candidatesTokensDetails"
        }
    ));
}

fn transform_usage_error(source: &str) -> StudioImageError {
    let raw: Box<RawValue> = serde_json::from_str(source).expect("usage source");
    let metadata = decode_image_usage(&raw).expect("the usage metadata decodes");
    transform_gemini_image_usage(&metadata).expect_err("the counts overflow")
}

#[test]
fn an_absent_usage_metadata_leaves_usage_away() {
    let response = generated(
        GEMINI,
        r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":"a"}}]}}]}"#,
    );
    assert_eq!(response.data.len(), 1);
    assert!(response.usage.is_none());
}

#[test]
fn a_null_usage_metadata_leaves_usage_away() {
    let response = generated(GEMINI, r#"{"candidates":[],"usageMetadata":null}"#);
    assert!(response.usage.is_none());
}

#[test]
fn an_empty_usage_metadata_makes_a_zero_usage() {
    let response = generated(GEMINI, r#"{"candidates":[],"usageMetadata":{}}"#);
    assert_eq!(response.usage, Some(ImageGenerationUsage::default()));
}

#[rstest]
#[case::text(r#""x""#)]
#[case::number("5")]
#[case::list("[]")]
#[case::boolean("true")]
fn a_non_object_usage_metadata_is_refused(#[case] metadata: &str) {
    let body = format!(r#"{{"usageMetadata":{metadata}}}"#);
    let error = transform_image_generation_response(GEMINI, &reply(&body))
        .expect_err("the usage type is declared as an object");
    assert!(matches!(error, StudioImageError::UsageShape));
}

#[test]
fn a_string_token_count_names_its_path() {
    let error = transform_image_generation_response(
        GEMINI,
        &reply(r#"{"usageMetadata":{"promptTokenCount":"4"}}"#),
    )
    .expect_err("the count type is declared as an integer");
    assert!(matches!(
        &error,
        StudioImageError::Shape { path, .. } if path == "promptTokenCount"
    ));
}

#[test]
fn every_inline_data_image_appends_with_its_truthy_signature() {
    let body = r#"{"candidates":[{"content":{"parts":[{"text":"here"},{"inlineData":{"data":"a"},"thoughtSignature":"sig"},{"inlineData":{"data":"b"}},{"inlineData":{"data":"c"},"thoughtSignature":""},{"inlineData":{"mimeType":"image/png"}}]}}]}"#;
    let response = generated(GEMINI, body);
    assert_eq!(
        serde_json::to_string(&response.data).expect("the images encode"),
        r#"[{"b64_json":"a","provider_specific_fields":{"thought_signature":"sig"}},{"b64_json":"b"},{"b64_json":"c"}]"#
    );
}

#[test]
fn an_inline_data_without_data_appends_no_image() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png"}},{"inlineData":{}}]}}]}"#;
    let response = generated(GEMINI, body);
    assert!(response.data.is_empty());
}

#[test]
fn a_null_inline_data_appends_no_image() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":null}}]}}]}"#;
    assert!(generated(GEMINI, body).data.is_empty());
}

#[test]
fn a_non_string_inline_data_names_its_shape() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":5}}]}}]}"#;
    let error = transform_image_generation_response(GEMINI, &reply(body))
        .expect_err("the data type is declared");
    assert!(matches!(
        &error,
        StudioImageError::Shape { path, .. } if path == "candidates[0].content.parts[0].inlineData.data"
    ));
}

#[test]
fn an_imagen_prediction_without_bytes_still_appends_a_null_image() {
    let body = r#"{"predictions":[{"bytesBase64Encoded":"aGk="},{},{"bytesBase64Encoded":null}]}"#;
    let response = generated(IMAGEN, body);
    assert_eq!(
        serde_json::to_string(&response.data).expect("the images encode"),
        r#"[{"b64_json":"aGk="},{},{}]"#
    );
    assert!(response.usage.is_none());
}

#[test]
fn a_non_string_prediction_image_names_its_path() {
    let error = transform_image_generation_response(
        IMAGEN,
        &reply(r#"{"predictions":[{"bytesBase64Encoded":5}]}"#),
    )
    .expect_err("the prediction type is declared");
    assert!(matches!(
        &error,
        StudioImageError::Shape { path, .. } if path == "predictions[0].bytesBase64Encoded"
    ));
}

#[rstest]
#[case::root("[]", "the document root")]
#[case::candidates(r#"{"candidates":{}}"#, "candidates")]
#[case::predictions(r#"{"predictions":{}}"#, "predictions")]
#[case::prediction(r#"{"predictions":["x"]}"#, "predictions[0]")]
fn a_non_object_reply_names_its_shape(#[case] body: &str, #[case] path: &str) {
    let error = decode_studio_image(&reply(body)).expect_err("the reply shape is declared");
    assert!(matches!(
        &error,
        StudioImageError::Shape { path: found, .. } if found == path
    ));
    assert!(error.upstream_answer().is_none());
}

#[test]
fn a_reply_that_is_not_json_keeps_the_upstream_status_and_headers() {
    let mut reply = reply(r#"{"candidates": ["#);
    reply.status = 502;
    reply
        .headers
        .insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = transform_image_generation_response(GEMINI, &reply)
        .expect_err("truncated JSON cannot become a response");
    assert!(matches!(error, StudioImageError::NotJson { .. }));
    let (status, headers) = error.upstream_answer().expect("the reply's answer is kept");
    assert_eq!(status, 502);
    assert_eq!(
        headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("req-1")
    );
}

#[test]
fn a_reply_at_the_limit_still_parses() {
    let mut reply = reply(r#"{"candidates":[]}"#);
    reply.body.resize(MAX_RESPONSE_BYTES, b' ');
    let response =
        transform_image_generation_response(GEMINI, &reply).expect("a body at the limit parses");
    assert_eq!(response, ImageGenerationResponse::default());
}

#[test]
fn a_reply_past_the_limit_is_refused_before_it_parses() {
    let mut reply = reply("");
    reply.body = vec![b'{'; MAX_RESPONSE_BYTES + 1];
    let error = decode_studio_image(&reply).expect_err("the body is past the limit");
    assert!(matches!(
        error,
        StudioImageError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
}

#[rstest]
fn replays_the_studio_image_corpus(#[files("fuzz/corpus/studio_image/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus file is readable");
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    };
    drop(decode_studio_image(&reply));
}

#[test]
fn the_search_count_rides_on_a_present_usage() {
    let body = r#"{"candidates":[{"content":{"parts":[]},"groundingMetadata":{"webSearchQueries":["a","b","a"]}}],"usageMetadata":{}}"#;
    let response = generated(GEMINI, body);
    let usage = response.usage.expect("the empty usage is still present");
    assert_eq!(usage.web_search_requests, Some(2));
    let uncounted = generated(
        GEMINI,
        r#"{"candidates":[{"content":{"parts":[]}}],"usageMetadata":{}}"#,
    );
    let usage = uncounted.usage.expect("the usage is present");
    assert_eq!(usage.web_search_requests, None);
    let encoded = serde_json::to_string(&usage).expect("the usage encodes");
    assert!(!encoded.contains("web_search_requests"));
    let ungrounded = generated(
        GEMINI,
        r#"{"candidates":[{"content":{"parts":[]},"groundingMetadata":{"webSearchQueries":["a"]}}]}"#,
    );
    assert!(ungrounded.usage.is_none());
}

#[test]
fn the_imagen_branch_reads_no_usage_and_no_candidates() {
    let body = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":"a"}}]}}],"predictions":[{"bytesBase64Encoded":"aGk="}],"usageMetadata":{"promptTokenCount":3}}"#;
    let response = generated(IMAGEN, body);
    assert_eq!(response.data.len(), 1);
    assert!(response.usage.is_none());
    let gemini = generated(GEMINI, body);
    assert_eq!(gemini.data.len(), 1);
    assert_eq!(gemini.usage.expect("the usage is read").input_tokens, 3);
}

// the injected pricing answers from fixed numbers, so every branch of the calculator shows in the total
struct Fake {
    catalog: PriceRecord,
    tokens: Option<Decimal>,
    known: bool,
}

impl Fake {
    fn new(catalog: &str) -> Self {
        Self {
            catalog: serde_json::from_str(catalog).expect("price record source"),
            tokens: None,
            known: true,
        }
    }
}

impl ImagePricing for Fake {
    fn model_info(
        &self,
        model: &str,
        provider: &str,
        _supplied: Option<&PriceRecord>,
    ) -> Result<PriceRecord, ImageCostError> {
        assert_eq!(provider, "gemini");
        if !self.known {
            return Err(ImageCostError::UnknownModel {
                model: model.to_owned(),
            });
        }
        Ok(self.catalog.clone())
    }

    fn web_search_cost(
        &self,
        provider: &str,
        web_search_requests: Option<i64>,
        _model_info: &PriceRecord,
    ) -> Decimal {
        assert_eq!(provider, "gemini");
        let count = web_search_requests.unwrap_or(0);
        Decimal::new(35, 3)
            .checked_mul(Decimal::from(count))
            .expect("the search fee fits")
    }

    fn usage_cost(
        &self,
        _model: &str,
        provider: &str,
        usage: &ImageUsageDetails,
        _model_info: &PriceRecord,
    ) -> Result<Option<Decimal>, ImageCostError> {
        assert_eq!(provider, "gemini");
        assert_eq!(usage.input_tokens, Some(4));
        assert_eq!(usage.output_tokens, Some(6));
        assert_eq!(usage.total_tokens, Some(10));
        assert_eq!(
            detail(usage.input_tokens_details.as_ref(), "text_tokens"),
            Some(1)
        );
        assert_eq!(
            detail(usage.input_tokens_details.as_ref(), "image_tokens"),
            Some(3)
        );
        assert_eq!(
            detail(usage.completion_tokens_details.as_ref(), "text_tokens"),
            Some(0)
        );
        assert_eq!(
            detail(usage.completion_tokens_details.as_ref(), "image_tokens"),
            Some(6)
        );
        assert_eq!(
            detail(usage.output_tokens_details.as_ref(), "image_tokens"),
            Some(6)
        );
        Ok(self.tokens)
    }
}

fn detail(details: Option<&DetailMap>, key: &str) -> Option<i64> {
    let raw = details?.get(key)?;
    serde_json::from_str(raw.get()).expect("a count is an integer")
}

fn billed_cost(
    response: &ImageGenerationResponse,
    fake: &Fake,
) -> Result<Decimal, StudioImageError> {
    generation_cost(fake, GEMINI, None, ImageCostResponse::Image(response))
}

fn priced_gemini(usage: &str) -> ImageGenerationResponse {
    let body = format!(
        r#"{{"candidates":[{{"content":{{"parts":[{{"inlineData":{{"data":"a"}}}},{{"inlineData":{{"data":"b"}}}}]}},"groundingMetadata":{{"webSearchQueries":["a","b"]}}}}],"usageMetadata":{usage}}}"#,
    );
    generated(GEMINI, &body)
}

#[test]
fn the_model_lookup_comes_before_the_response_type() {
    let mut missing = Fake::new(r#"{"output_cost_per_image":0.04}"#);
    missing.known = false;
    let error = generation_cost(&missing, GEMINI, None, ImageCostResponse::Other)
        .expect_err("the model is looked up first");
    assert!(matches!(
        error,
        StudioImageError::Cost(ImageCostError::UnknownModel { .. })
    ));
    let error = generation_cost(&Fake::new("{}"), GEMINI, None, ImageCostResponse::Other)
        .expect_err("a non-image response is refused");
    assert!(matches!(
        error,
        StudioImageError::Cost(ImageCostError::NotImageResponse)
    ));
}

#[test]
fn a_token_cost_replaces_the_per_image_price_and_search_is_added() {
    let mut fake = Fake::new(r#"{"output_cost_per_image":0.04}"#);
    fake.tokens = Some(Decimal::new(5, 1));
    let response = priced_gemini(CANONICAL_USAGE);
    assert_eq!(response.data.len(), 2);
    assert_eq!(
        billed_cost(&response, &fake).expect("priced"),
        Decimal::new(57, 2)
    );
}

#[test]
fn a_zero_token_cost_suppresses_the_per_image_fallback() {
    let mut fake = Fake::new(r#"{"output_cost_per_image":0.04}"#);
    fake.tokens = Some(Decimal::ZERO);
    assert_eq!(
        billed_cost(&priced_gemini(CANONICAL_USAGE), &fake).expect("priced"),
        Decimal::ZERO
    );
}

#[test]
fn an_unpriced_token_table_bills_each_returned_image() {
    let fake = Fake::new(r#"{"output_cost_per_image":0.04}"#);
    let response = priced_gemini(CANONICAL_USAGE);
    assert_eq!(response.data.len(), 2);
    assert_eq!(
        billed_cost(&response, &fake).expect("priced"),
        Decimal::new(15, 2)
    );
}

#[test]
fn an_absent_usage_bills_the_images_and_skips_the_search_count() {
    let fake = Fake::new(r#"{"output_cost_per_image":0.04}"#);
    let body = r#"{"candidates":[{"content":{"parts":[]},"groundingMetadata":{"webSearchQueries":["a"]}}]}"#;
    let response = generated(GEMINI, body);
    assert_eq!(
        billed_cost(&response, &fake).expect("priced"),
        Decimal::new(4, 2)
    );
}

#[test]
fn a_catalog_without_an_image_price_costs_nothing() {
    let fake = Fake::new("{}");
    let body = r#"{"predictions":[{"bytesBase64Encoded":"a"}]}"#;
    assert_eq!(
        billed_cost(&generated(IMAGEN, body), &fake).expect("priced"),
        Decimal::ZERO
    );
}
