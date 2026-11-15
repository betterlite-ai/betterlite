#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::vertex_schema_shaping::{Raw, Settings};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{HeaderMap, HeaderValue};
use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::decode::decode_google_image;
use super::error::GeminiImageError;
use super::options::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use super::request::transform_image_generation_request;
use super::response::{ImageGenerationResponse, transform_image_generation_response};
use crate::gemini_response::decode::MAX_RESPONSE_BYTES;

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("object source")
}

fn json_value(source: &str) -> Json {
    serde_json::from_str(source).expect("json source")
}

fn wrapped(params: &Object) -> Json {
    Json::Object(params.clone())
}

fn settings() -> Settings {
    Raw::default()
        .convert(&BTreeMap::<String, String>::new())
        .expect("empty settings")
}

fn reply(body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

fn encoded(response: &ImageGenerationResponse) -> String {
    serde_json::to_string(response).expect("the response encodes")
}

#[test]
fn the_supported_params_are_the_openai_pair_and_the_native_spellings() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        [
            "n",
            "size",
            "imageConfig",
            "aspectRatio",
            "aspect_ratio",
            "imageSize",
            "image_size",
            "tools",
            "web_search_options",
        ]
    );
}

#[test]
fn a_known_size_asks_for_its_aspect_ratio_and_an_unknown_one_for_a_square() {
    let wide = map_openai_params(
        &object(r#"{"size":"1792x1024"}"#),
        &object("{}"),
        &settings(),
    )
    .expect("a string size maps");
    assert_eq!(
        wrapped(&wide.params),
        json_value(r#"{"aspectRatio":"16:9"}"#)
    );
    assert_eq!(wide.warnings, 0);
    let unknown = map_openai_params(&object(r#"{"size":"999x999"}"#), &object("{}"), &settings())
        .expect("an unknown size still maps");
    assert_eq!(
        wrapped(&unknown.params),
        json_value(r#"{"aspectRatio":"1:1"}"#)
    );
}

#[test]
fn a_mapped_key_never_overrides_a_given_optional_param() {
    let mapped = map_openai_params(&object(r#"{"n":2}"#), &object(r#"{"n":5}"#), &settings())
        .expect("the given value wins");
    assert_eq!(wrapped(&mapped.params), json_value("{}"));
    let mapped = map_openai_params(&object(r#"{"n":2}"#), &object("{}"), &settings())
        .expect("n maps to candidate_count");
    assert_eq!(
        wrapped(&mapped.params),
        json_value(r#"{"candidate_count":2}"#)
    );
}

#[test]
fn both_flat_spellings_land_on_the_native_keys() {
    let snake = map_openai_params(
        &object(r#"{"aspect_ratio":"4:3","image_size":"2K"}"#),
        &object("{}"),
        &settings(),
    )
    .expect("the snake spellings map");
    assert_eq!(
        wrapped(&snake.params),
        json_value(r#"{"aspectRatio":"4:3","imageSize":"2K"}"#)
    );
    let camel = map_openai_params(
        &object(r#"{"aspectRatio":"3:4","imageSize":"1K"}"#),
        &object("{}"),
        &settings(),
    )
    .expect("the camel spellings map");
    assert_eq!(
        wrapped(&camel.params),
        json_value(r#"{"aspectRatio":"3:4","imageSize":"1K"}"#)
    );
}

#[rstest]
#[case("null")]
#[case(r#""1K""#)]
#[case("3")]
#[case("[]")]
#[case("true")]
fn a_non_object_image_config_is_dropped_with_a_warning(#[case] value: &str) {
    let source = format!(r#"{{"imageConfig":{value}}}"#);
    let mapped = map_openai_params(&object(&source), &object("{}"), &settings())
        .expect("a non-object imageConfig is ignored");
    assert_eq!(wrapped(&mapped.params), json_value("{}"), "{value}");
    assert_eq!(mapped.warnings, 1, "{value}");
}

// the warning goes through LiteLLM's logger, so the captured fmt line carries its exact text
fn warning_text(value: &str) -> String {
    let log = NamedTempFile::new().expect("the log file opens");
    let writer = Arc::new(log.reopen().expect("the log reopens"));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer)
        .finish();
    let source = format!(r#"{{"imageConfig":{value}}}"#);
    with_default(subscriber, || {
        let mapped = map_openai_params(&object(&source), &object("{}"), &settings())
            .expect("a non-object imageConfig is ignored");
        assert_eq!(mapped.warnings, 1);
        fs::read_to_string(log.path()).expect("the log is readable")
    })
}

#[rstest]
#[case("null", "NoneType")]
#[case("true", "bool")]
#[case("3", "int")]
#[case("-3", "int")]
#[case("1.5", "float")]
#[case(r#""1K""#, "str")]
#[case("[]", "list")]
fn a_non_object_image_config_names_the_python_type_it_dropped(
    #[case] value: &str,
    #[case] name: &str,
) {
    let log = warning_text(value);
    assert!(
        log.contains(&format!("got {name} — ignoring.")),
        "{value} should warn with {name}: {log}"
    );
}

#[test]
fn a_size_that_is_not_a_string_names_the_field() {
    let error = map_openai_params(&object(r#"{"size":1024}"#), &object("{}"), &settings())
        .expect_err("a number is not a size");
    assert!(matches!(
        error,
        GeminiImageError::FieldType { param: "size", .. }
    ));
}

#[test]
fn the_tools_mapping_runs_after_the_plain_keys() {
    let mapped = map_openai_params(
        &object(r#"{"web_search_options":{"search_context_size":"low"}}"#),
        &object("{}"),
        &settings(),
    )
    .expect("the search options become a tool");
    assert_eq!(
        wrapped(&mapped.params),
        json_value(r#"{"tools":[{"googleSearch":{}}]}"#)
    );
}

#[test]
fn the_body_is_one_user_turn_asking_for_image_parts() {
    let body =
        transform_image_generation_request("a cat", &object("{}")).expect("the prompt transforms");
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"contents":[{"role":"user","parts":[{"text":"a cat"}]}],"generationConfig":{"responseModalities":["IMAGE"]}}"#
        )
    );
}

#[test]
fn the_flat_ratio_overwrites_the_nested_config_and_keeps_its_other_keys() {
    let params =
        object(r#"{"imageConfig":{"aspectRatio":"1:1","imageSize":"1K"},"aspect_ratio":"16:9"}"#);
    let body = transform_image_generation_request("p", &params).expect("the params transform");
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"contents":[{"role":"user","parts":[{"text":"p"}]}],"generationConfig":{"responseModalities":["IMAGE"],"imageConfig":{"aspectRatio":"16:9","imageSize":"1K"}}}"#
        )
    );
}

#[test]
fn candidate_count_wins_over_n_and_a_falsy_tools_list_is_dropped() {
    let params = object(
        r#"{"candidate_count":3,"n":7,"tools":[],"toolConfig":{"functionCallingConfig":{"mode":"NONE"}}}"#,
    );
    let body = transform_image_generation_request("p", &params).expect("the params transform");
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"contents":[{"role":"user","parts":[{"text":"p"}]}],"generationConfig":{"responseModalities":["IMAGE"],"candidateCount":3},"toolConfig":{"functionCallingConfig":{"mode":"NONE"}}}"#
        )
    );
}

#[test]
fn an_empty_image_config_never_reaches_the_generation_config() {
    let body = transform_image_generation_request("p", &object(r#"{"imageConfig":{}}"#))
        .expect("the empty config transforms");
    assert_eq!(
        wrapped(&body),
        json_value(
            r#"{"contents":[{"role":"user","parts":[{"text":"p"}]}],"generationConfig":{"responseModalities":["IMAGE"]}}"#
        )
    );
}

#[test]
fn a_non_object_image_config_names_the_field() {
    let error = transform_image_generation_request("p", &object(r#"{"imageConfig":"1K"}"#))
        .expect_err("a string is not an object");
    assert!(matches!(
        error,
        GeminiImageError::FieldType {
            param: "imageConfig",
            ..
        }
    ));
}

#[test]
fn every_inline_data_is_appended_with_its_signature() {
    let response = transform_image_generation_response(&reply(
        r#"{"candidates":[{"content":{"parts":[{"text":"draft"},{"inlineData":{"mimeType":"image/png","data":"AAA"},"thoughtSignature":"sig"},{"inlineData":{"data":"BBB"}}]}},{"content":{"parts":[{"inlineData":{"mimeType":"image/jpeg","data":"CCC"}}]}}]}"#,
    ))
    .expect("the reply transforms");
    assert_eq!(
        encoded(&response),
        r#"{"data":[{"b64_json":"AAA","provider_specific_fields":{"thought_signature":"sig"}},{"b64_json":"BBB"},{"b64_json":"CCC"}]}"#
    );
}

#[test]
fn a_blank_signature_is_not_attached() {
    let response = transform_image_generation_response(&reply(
        r#"{"candidates":[{"content":{"parts":[{"inlineData":{"data":"AAA"},"thoughtSignature":""}]}}]}"#,
    ))
    .expect("the reply transforms");
    assert_eq!(encoded(&response), r#"{"data":[{"b64_json":"AAA"}]}"#);
}

#[test]
fn usage_sums_only_text_and_image_details() {
    let response = transform_image_generation_response(&reply(
        r#"{"usageMetadata":{"promptTokenCount":9,"promptTokensDetails":[{"modality":"TEXT","tokenCount":4},{"modality":"IMAGE","tokenCount":5},{"modality":"AUDIO","tokenCount":7},{"modality":"","tokenCount":8},{"tokenCount":6}],"candidatesTokenCount":132,"totalTokenCount":141}}"#,
    ))
    .expect("the usage transforms");
    assert_eq!(
        encoded(&response),
        r#"{"data":[],"usage":{"input_tokens":9,"input_tokens_details":{"image_tokens":5,"text_tokens":4},"output_tokens":132,"total_tokens":141}}"#
    );
}

#[test]
fn an_absent_token_count_is_zero() {
    let response = transform_image_generation_response(&reply(
        r#"{"usageMetadata":{"promptTokensDetails":[{"modality":"IMAGE","tokenCount":2}]}}"#,
    ))
    .expect("an absent count is zero");
    assert_eq!(
        encoded(&response),
        r#"{"data":[],"usage":{"input_tokens":0,"input_tokens_details":{"image_tokens":2,"text_tokens":0},"output_tokens":0,"total_tokens":0}}"#
    );
}

#[test]
fn the_search_count_rides_the_usage_only_when_usage_exists() {
    let searched = r#""candidates":[{"groundingMetadata":{"webSearchQueries":["a","b"]}},{"groundingMetadata":[{"webSearchQueries":["b"]}]}]"#;
    let without = transform_image_generation_response(&reply(&format!("{{{searched}}}")))
        .expect("the reply transforms");
    assert_eq!(without.usage, None);
    let empty = transform_image_generation_response(&reply(&format!(
        "{{{searched},\"usageMetadata\":{{}}}}"
    )))
    .expect("an empty usage is falsy");
    assert_eq!(empty.usage, None);
    let counted = transform_image_generation_response(&reply(&format!(
        "{{{searched},\"usageMetadata\":{{\"promptTokenCount\":1}}}}"
    )))
    .expect("the search count attaches");
    assert_eq!(
        counted.usage.and_then(|usage| usage.web_search_requests),
        Some(2)
    );
}

#[test]
fn a_truthy_usage_that_is_not_an_object_cannot_transform() {
    let error = transform_image_generation_response(&reply(r#"{"usageMetadata":"x"}"#))
        .expect_err("a string is not a usage object");
    assert!(matches!(error, GeminiImageError::UsageShape));
}

#[test]
fn a_token_count_that_is_not_an_integer_names_the_field() {
    let error = transform_image_generation_response(&reply(
        r#"{"usageMetadata":{"promptTokenCount":"9"}}"#,
    ))
    .expect_err("a string is not a count");
    assert!(matches!(
        error,
        GeminiImageError::Shape { ref path, .. } if path == "promptTokenCount"
    ));
}

#[test]
fn an_usage_object_of_only_nulls_is_present_with_zero_counts() {
    let response = transform_image_generation_response(&reply(
        r#"{"usageMetadata":{"promptTokenCount":null}}"#,
    ))
    .expect("nulls take their default");
    assert_eq!(
        encoded(&response),
        r#"{"data":[],"usage":{"input_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0},"output_tokens":0,"total_tokens":0}}"#
    );
}

#[test]
fn token_counts_that_leave_the_signed_range_are_refused() {
    let body = format!(
        r#"{{"usageMetadata":{{"promptTokensDetails":[{{"modality":"TEXT","tokenCount":{}}},{{"modality":"TEXT","tokenCount":{}}}]}}}}"#,
        i64::MAX,
        i64::MAX
    );
    let error = transform_image_generation_response(&reply(&body))
        .expect_err("the text token sum overflows");
    assert!(matches!(
        error,
        GeminiImageError::TokenOverflow {
            field: "promptTokensDetails"
        }
    ));
}

#[test]
fn a_body_that_is_not_json_keeps_the_upstream_status_and_headers() {
    let mut reply = reply(r#"{"candidates": ["#);
    reply.status = 502;
    reply
        .headers
        .insert("x-request-id", HeaderValue::from_static("req-1"));
    let error = transform_image_generation_response(&reply)
        .expect_err("truncated JSON cannot become a response");
    assert!(matches!(error, GeminiImageError::NotJson { .. }));
    assert_eq!(error.status_code(), Some(502));
    assert_eq!(
        error
            .headers()
            .and_then(|headers| headers.get("x-request-id"))
            .and_then(|value| value.to_str().ok()),
        Some("req-1")
    );
}

#[test]
fn an_array_root_or_a_non_list_candidates_names_its_shape() {
    let error = decode_google_image(&reply("[]")).expect_err("an array is not a reply object");
    assert!(matches!(
        error,
        GeminiImageError::Shape { ref path, .. } if path == "the document root"
    ));
    let error = decode_google_image(&reply(r#"{"candidates": {}}"#))
        .expect_err("candidates must be a list");
    assert!(matches!(
        error,
        GeminiImageError::Shape { ref path, .. } if path == "candidates"
    ));
    assert_eq!(error.status_code(), None);
}

#[test]
fn a_reply_at_the_limit_still_parses() {
    let mut reply = reply(r#"{"candidates":[]}"#);
    reply.body.resize(MAX_RESPONSE_BYTES, b' ');
    let response = transform_image_generation_response(&reply).expect("a body at the limit parses");
    assert_eq!(response, ImageGenerationResponse::default());
}

#[test]
fn a_reply_past_the_limit_is_refused_before_it_parses() {
    let mut reply = reply("");
    reply.body = vec![b'{'; MAX_RESPONSE_BYTES + 1];
    let error = decode_google_image(&reply).expect_err("the body is past the limit");
    assert!(matches!(
        error,
        GeminiImageError::TooLarge { size } if size == MAX_RESPONSE_BYTES + 1
    ));
}

#[test]
fn null_fields_take_their_defaults() {
    let response =
        transform_image_generation_response(&reply(r#"{"candidates":null,"usageMetadata":null}"#))
            .expect("nulls take the default");
    assert_eq!(response, ImageGenerationResponse::default());
}

#[rstest]
fn replays_the_google_image_corpus(#[files("fuzz/corpus/google_image/*")] path: PathBuf) {
    let body = fs::read(path).expect("the corpus file is readable");
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body,
    };
    drop(transform_image_generation_response(&reply));
}
