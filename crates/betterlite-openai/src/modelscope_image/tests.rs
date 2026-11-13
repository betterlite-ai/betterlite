#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::{ErrorClass, ModelScopeImageError, error_class};
use super::params::{SUPPORTED_OPENAI_PARAMS, map_openai_params};
use crate::image_generation::reply::{ImageGenerationReply, ImageUsage};

use super::reply::{ImageReply, decode_image_generation_reply};
use super::request::{
    DEFAULT_BASE_URL, get_complete_url, image_request_body, validate_environment,
};
use crate::openai_base_config::{Headers, Object};

const CREATED: i64 = 1_700_000_000;

fn object(value: Value) -> Object {
    let Value::Object(fields) = value else {
        panic!("the fixture must be a JSON object");
    };
    fields
}

fn decode(body: &str, status: u16) -> Result<ImageGenerationReply, ModelScopeImageError> {
    decode_image_generation_reply(
        ImageReply {
            status,
            body: body.as_bytes(),
        },
        CREATED,
    )
}

#[test]
fn advertises_the_four_openai_options() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["n", "response_format", "size", "user"]
    );
}

#[test]
fn copies_the_supported_options() {
    let params = object(json!({"n": 2, "size": "1024x1024", "user": "abc"}));
    let mut optional = Object::new();
    map_openai_params(&params, &mut optional, "Qwen", false).expect("the options are supported");
    assert_eq!(optional, params);
}

#[test]
fn copies_other_keys_when_drop_params_is_set() {
    let params = object(json!({"n": 1, "quality": "hd", "style": "vivid"}));
    let mut optional = Object::new();
    map_openai_params(&params, &mut optional, "Qwen", true).expect("drop_params drops the rest");
    assert_eq!(optional, object(json!({"n": 1})));
}

#[test]
fn rejects_an_unsupported_option_unless_it_is_dropped() {
    let params = object(json!({"quality": "hd"}));
    let mut optional = Object::new();
    let error = map_openai_params(&params, &mut optional, "Qwen", false)
        .expect_err("quality is not a ModelScope option");
    assert_eq!(error.status_code(), Some(400));
    assert!(error.to_string().contains("quality"));
}

#[test]
fn an_earlier_option_is_replaced_by_the_same_name() {
    let mut optional = object(json!({"n": 1}));
    map_openai_params(&object(json!({"n": 3})), &mut optional, "Qwen", false)
        .expect("n is supported");
    assert_eq!(optional["n"], 3);
}

#[rstest]
#[case::default(
    None,
    None,
    "https://api-inference.modelscope.cn/v1/images/generations"
)]
#[case::environment(
    None,
    Some("https://env.example/v1"),
    "https://env.example/v1/images/generations"
)]
#[case::api_base(
    Some("https://base.example/v1"),
    Some("https://env.example"),
    "https://base.example/v1/images/generations"
)]
#[case::trailing_slash(
    Some("https://base.example/v1///"),
    None,
    "https://base.example/v1/images/generations"
)]
fn builds_the_url(
    #[case] api_base: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(get_complete_url(api_base, environment_base), expected);
    assert!(expected.ends_with("/images/generations"));
}

#[test]
fn the_default_base_is_the_modelscope_inference_host() {
    assert_eq!(DEFAULT_BASE_URL, "https://api-inference.modelscope.cn/v1");
}

#[test]
fn the_api_key_wins_over_the_environment_key_and_sets_the_headers() {
    let key = SecretString::from("direct");
    let environment = SecretString::from("environment");
    let headers = validate_environment(&Headers::new(), Some(&key), Some(&environment))
        .expect("a key is set");
    assert_eq!(headers["Authorization"], "Bearer direct");
    assert_eq!(headers["Content-Type"], "application/json");
}

#[test]
fn the_environment_key_is_used_when_no_api_key_is_passed() {
    let environment = SecretString::from("environment");
    let headers =
        validate_environment(&Headers::new(), None, Some(&environment)).expect("a key is set");
    assert_eq!(headers["Authorization"], "Bearer environment");
}

#[test]
fn a_missing_key_names_the_environment_variable() {
    let error = validate_environment(&Headers::new(), None, None).expect_err("no key is set");
    assert_missing_key(&error);
}

#[test]
fn an_empty_api_key_falls_to_the_environment_key_and_then_to_the_error() {
    let empty = SecretString::from("");
    let environment = SecretString::from("environment");
    let headers = validate_environment(&Headers::new(), Some(&empty), Some(&environment))
        .expect("the environment key is set");
    assert_eq!(headers["Authorization"], "Bearer environment");
    let error = validate_environment(&Headers::new(), Some(&empty), Some(&empty))
        .expect_err("both keys are empty");
    assert_missing_key(&error);
}

fn assert_missing_key(error: &ModelScopeImageError) {
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("MODELSCOPE_API_KEY is not set"));
}

#[test]
fn the_caller_headers_stay_beside_the_two_that_win() {
    let key = SecretString::from("direct");
    let mut given = Headers::new();
    given.insert(String::from("X-Trace"), String::from("abc"));
    given.insert(String::from("Authorization"), String::from("Bearer old"));
    let headers = validate_environment(&given, Some(&key), None).expect("a key is set");
    assert_eq!(headers["X-Trace"], "abc");
    assert_eq!(headers["Authorization"], "Bearer direct");
    assert_eq!(headers.len(), 3);
}

#[test]
fn builds_the_body_from_the_model_prompt_and_options() {
    let options = object(json!({"n": 2, "size": "1024x1024", "user": null}));
    let body = image_request_body("Qwen/Qwen-Image", "a cat", &options);
    assert_eq!(
        Value::Object(body),
        json!({
            "model": "Qwen/Qwen-Image",
            "prompt": "a cat",
            "n": 2,
            "size": "1024x1024",
            "user": null
        })
    );
}

#[test]
fn the_body_drops_options_whose_name_starts_with_an_underscore() {
    let options = object(json!({"n": 1, "_internal": "hidden", "_": true, "user": "abc"}));
    let body = image_request_body("Qwen", "a cat", &options);
    assert!(!body.contains_key("_internal"));
    assert!(!body.contains_key("_"));
    assert_eq!(body["user"], "abc");
}

#[test]
fn decodes_the_data_items() {
    let reply = decode(
        r#"{"created": 5, "data": [{"url": "https://example/1.png", "b64_json": null, "revised_prompt": "a cat"}, {"url": "https://example/2.png"}]}"#,
        200,
    )
    .expect("the reply carries images");
    assert_eq!(reply.created, CREATED);
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.data[0].url.as_deref(), Some("https://example/1.png"));
    assert_eq!(reply.data[0].b64_json, None);
    assert_eq!(reply.data[0].revised_prompt.as_deref(), Some("a cat"));
    assert_eq!(reply.data[1].url.as_deref(), Some("https://example/2.png"));
    assert_eq!(reply.data[1].revised_prompt, None);
    assert_eq!(reply.usage, ImageUsage::default());
}

#[test]
fn a_reply_without_data_has_no_images() {
    let reply = decode(r#"{"created": 5}"#, 200).expect("a reply without data is fine");
    assert!(reply.data.is_empty());
}

#[test]
fn a_null_data_field_has_no_images() {
    let reply = decode(r#"{"data": null}"#, 200).expect("a null data field takes the default");
    assert!(reply.data.is_empty());
}

#[rstest]
#[case::bad_request(400, ErrorClass::BadRequest)]
#[case::authentication(401, ErrorClass::Authentication)]
#[case::server(500, ErrorClass::Server)]
#[case::server_above(503, ErrorClass::Server)]
#[case::other(403, ErrorClass::BadRequest)]
fn picks_the_error_class_by_status(#[case] status: u16, #[case] expected: ErrorClass) {
    assert_eq!(error_class(status), expected);
}

#[test]
fn each_error_class_carries_litellms_status() {
    assert_eq!(ErrorClass::BadRequest.status(), 400);
    assert_eq!(ErrorClass::Authentication.status(), 401);
    assert_eq!(ErrorClass::Server.status(), 500);
}

#[rstest]
#[case::bad_request(400, 400)]
#[case::authentication(401, 401)]
#[case::server(502, 500)]
#[case::other(403, 400)]
fn a_body_that_is_not_json_keeps_the_class_of_its_status(
    #[case] raw_status: u16,
    #[case] expected: u16,
) {
    let error = decode("{not json", raw_status).expect_err("the reply is not JSON");
    assert_eq!(error.status_code(), Some(expected));
}

#[test]
fn the_error_key_names_the_class_of_the_raw_status() {
    let reply = decode(r#"{"error": {"message": "invalid key"}}"#, 401)
        .expect_err("the reply reports an error");
    assert_eq!(reply.status_code(), Some(401));
    assert!(reply.to_string().contains("ModelScope error: invalid key"));
}

#[test]
fn an_error_key_on_an_unknown_status_is_a_bad_request() {
    let reply =
        decode(r#"{"error": {"message": "boom"}}"#, 422).expect_err("the reply reports an error");
    assert_eq!(reply.status_code(), Some(400));
}

#[test]
fn a_null_error_key_is_a_wrong_typed_field() {
    let error = decode(r#"{"error": null}"#, 401).expect_err("a null error key cannot be read");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn a_reply_that_is_not_an_object_is_an_error() {
    let error = decode("[1, 2]", 200).expect_err("a list is not a reply");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn an_item_that_is_not_an_object_is_an_error() {
    let error = decode(r#"{"data": ["x"]}"#, 200).expect_err("an item must be an object");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn a_data_field_that_is_not_a_list_is_an_error() {
    let error = decode(r#"{"data": {}}"#, 200).expect_err("data must be a list");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn an_image_field_of_the_wrong_type_is_an_error() {
    let error = decode(r#"{"data": [{"url": 5}]}"#, 200).expect_err("url must be a string");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn an_empty_reply_body_is_an_error() {
    let error = decode("", 500).expect_err("an empty body is not JSON");
    assert_eq!(error.status_code(), Some(500));
}

#[test]
fn replays_the_modelscope_image_reply_corpus() {
    let corpus =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/modelscope_image_reply");
    let mut paths: Vec<PathBuf> = fs::read_dir(corpus)
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no ModelScope image corpus seeds");
    paths.into_iter().for_each(|path| {
        let bytes = fs::read(&path).expect("the corpus seed reads");
        let _reply = decode_image_generation_reply(
            ImageReply {
                status: 200,
                body: &bytes,
            },
            CREATED,
        );
    });
}
