#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use betterlite_tokenizer::engine::{Engine, assets};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;

use super::request::{
    get_complete_url, get_error_class, map_openai_params, transform_embedding_request,
    transform_request, validate_environment,
};
use super::response::{CohereEmbeddingResponse, DecodeInput, transform_embedding_response};
use super::usage::calculate_usage;
use super::{
    CohereEmbeddingError, DEFAULT_COMPLETE_URL, DEFAULT_EMBEDDING_INPUT_TYPE, EmbeddingLogging,
    PostCall, SUPPORTED_OPENAI_PARAMS, TextEncoding,
};
use crate::openai_base_config::Headers;

#[derive(Serialize)]
struct Recorded<'call> {
    input: &'call [String],
    api_key: Option<String>,
    complete_input_dict: &'call Object,
    original_response: &'call Json,
}

struct Fixture {
    encoded: Mutex<Vec<String>>,
    fail_on: Option<String>,
    input: Vec<String>,
    calls: Vec<serde_json::Value>,
}

impl TextEncoding for Fixture {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        let Ok(mut encoded) = self.encoded.lock() else {
            return Err(EngineError::PoolClosed);
        };
        encoded.push(text.to_owned());
        if self.fail_on.as_deref() == Some(text) {
            return Err(EngineError::InputTooLarge {
                bytes: text.len(),
                limit: 0,
            });
        }
        Ok(text.chars().count())
    }
}

impl EmbeddingLogging for Fixture {
    fn input(&self) -> &[String] {
        &self.input
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        let recorded = Recorded {
            input: call.input,
            api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
            complete_input_dict: call.complete_input_dict,
            original_response: call.original_response,
        };
        self.calls.push(serde_json::to_value(recorded).unwrap());
    }
}

fn fixture(fail_on: Option<&str>, input: &[&str]) -> Fixture {
    Fixture {
        encoded: Mutex::new(Vec::new()),
        fail_on: fail_on.map(str::to_owned),
        input: texts(input),
        calls: Vec::new(),
    }
}

fn params(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[rstest]
#[case::text(r#"["hello"]"#, "texts", "search_document")]
#[case::image_last(r#"["hello","data:image/png;base64,YWJj"]"#, "images", "image")]
#[case::text_last(
    r#"["data:image/png;base64,YWJj","hello"]"#,
    "texts",
    "search_document"
)]
fn the_last_input_decides_the_modality(
    #[case] source: &str,
    #[case] key: &str,
    #[case] input_type: &str,
) {
    let input: Vec<String> = serde_json::from_str(source).unwrap();
    let request = transform_request("m", &input, &Object::new());
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["model"], "m");
    assert_eq!(value["input_type"], input_type);
    assert_eq!(value[key], serde_json::json!(input));
    assert_eq!(value.as_object().unwrap().len(), 3);
}

#[test]
fn the_wrapper_wraps_a_string_and_the_lower_request_accepts_an_empty_list() {
    let wrapped = transform_embedding_request(
        "m",
        &serde_json::from_str(r#""hello""#).unwrap(),
        &Object::new(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&wrapped).unwrap()["texts"],
        serde_json::json!(["hello"])
    );
    let empty = serde_json::to_value(transform_request("m", &[], &Object::new())).unwrap();
    assert_eq!(empty["texts"], serde_json::json!([]));
    assert_eq!(empty["input_type"], DEFAULT_EMBEDDING_INPUT_TYPE);
    assert!(matches!(
        transform_embedding_request("m", &serde_json::from_str("[]").unwrap(), &Object::new()),
        Err(CohereEmbeddingError::EmptyInput)
    ));
}

#[test]
fn the_wrapper_passes_a_non_empty_list_through() {
    let request = transform_embedding_request(
        "m",
        &serde_json::from_str(r#"["a","b"]"#).unwrap(),
        &Object::new(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&request).unwrap()["texts"],
        serde_json::json!(["a", "b"])
    );
}

#[rstest]
#[case::tokens("[1, 2]")]
#[case::token_lists("[[1], [2]]")]
fn token_inputs_are_refused_by_the_wrapper(#[case] source: &str) {
    let refused =
        transform_embedding_request("m", &serde_json::from_str(source).unwrap(), &Object::new());
    assert!(matches!(refused, Err(CohereEmbeddingError::InputNotText)));
}

#[test]
fn inference_parameters_overlay_the_built_request() {
    let overlay = r#"{"model":"other","texts":["over"],"images":["pic"],"input_type":"clustering","truncate":"START","embedding_types":["int8"],"output_dimension":64,"max_tokens":3}"#;
    let request = transform_request("m", &texts(&["hello"]), &params(overlay));
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::from_str::<serde_json::Value>(overlay).unwrap()
    );
    let keys: Vec<&str> = request.keys().map(String::as_str).collect();
    assert_eq!(&keys[..3], ["model", "texts", "input_type"]);
}

#[test]
fn a_null_inference_value_overlays_and_is_sent_as_null() {
    let source = r#"{"model":null,"input_type":null,"texts":null,"truncate":null,"output_dimension":null,"extra":null}"#;
    let request = transform_request("m", &texts(&["hello"]), &params(source));
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        serde_json::from_str::<serde_json::Value>(source).unwrap()
    );
}

#[rstest]
#[case::scalar(r#""float""#, r#"["float"]"#)]
#[case::list(r#"["float","int8"]"#, r#"["float","int8"]"#)]
#[case::null("null", "[null]")]
fn encoding_format_becomes_embedding_types(#[case] value: &str, #[case] expected: &str) {
    let mapped = map_openai_params(
        &params(&format!(r#"{{"encoding_format":{value}}}"#)),
        Object::new(),
    );
    assert_eq!(mapped.get("embedding_types"), Some(&json(expected)));
}

#[test]
fn the_wire_defaults_match_litellm() {
    assert_eq!(SUPPORTED_OPENAI_PARAMS, ["encoding_format", "dimensions"]);
    assert_eq!(DEFAULT_EMBEDDING_INPUT_TYPE, "search_document");
    let mapped = map_openai_params(
        &params(r#"{"dimensions":256,"user":"reader"}"#),
        Object::new(),
    );
    assert_eq!(mapped.get("output_dimension"), Some(&json("256")));
    assert_eq!(mapped.len(), 1);
    assert_eq!(get_complete_url(None), DEFAULT_COMPLETE_URL);
    assert_eq!(get_complete_url(Some("")), DEFAULT_COMPLETE_URL);
    assert_eq!(
        get_complete_url(Some("https://h/v2/embed")),
        "https://h/v2/embed"
    );
}

#[test]
fn a_caller_authorization_beats_the_key_bearer() {
    let headers = validate_environment(
        Headers::from([(String::from("Authorization"), String::from("Bearer caller"))]),
        Some(&SecretString::from("sk-test")),
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer caller")
    );
}

#[test]
fn the_error_class_keeps_the_status_and_drops_the_headers() {
    let sent = Headers::from([(String::from("retry-after"), String::from("7"))]);
    let error = get_error_class(String::from("slow down"), 429, &sent);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert!(error.headers.is_none());
}

#[rstest]
#[case::key(Some("sk-test"), Some("Bearer sk-test"))]
#[case::empty_key(Some(""), None)]
#[case::no_key(None, None)]
fn the_default_headers_lose_to_the_caller_spelling(
    #[case] api_key: Option<&str>,
    #[case] authorization: Option<&str>,
) {
    let key = api_key.map(SecretString::from);
    let headers = validate_environment(
        Headers::from([(String::from("Content-Type"), String::from("text/plain"))]),
        key.as_ref(),
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        authorization
    );
}

#[rstest]
#[case::both(r#"{"input_tokens":5,"images":2}"#, 7, Some(5), Some(2))]
#[case::zeroes(r#"{"input_tokens":0,"images":0}"#, 0, Some(0), Some(0))]
#[case::text_only(r#"{"input_tokens":3}"#, 3, Some(3), None)]
#[case::images_only(r#"{"images":4}"#, 4, None, Some(4))]
#[case::stray_image_tokens(
    r#"{"input_tokens":0,"images":1,"image_tokens":1000}"#,
    1,
    Some(0),
    Some(1)
)]
fn billed_units_sum_and_keep_both_details(
    #[case] billed: &str,
    #[case] prompt: i64,
    #[case] text_tokens: Option<i64>,
    #[case] image_tokens: Option<i64>,
) {
    let fixture = fixture(None, &["hello"]);
    let meta = Json::Object(params(&format!(r#"{{"billed_units":{billed}}}"#)));
    let usage = calculate_usage(&texts(&["hello"]), &fixture, Some(&meta)).unwrap();
    assert_eq!(
        (
            usage.prompt_tokens,
            usage.total_tokens,
            usage.completion_tokens
        ),
        (prompt, prompt, 0)
    );
    assert_eq!(
        usage.prompt_tokens_details.unwrap().text_tokens,
        text_tokens
    );
    assert_eq!(
        usage.prompt_tokens_details.unwrap().image_tokens,
        image_tokens
    );
    assert!(fixture.encoded.lock().unwrap().is_empty());
}

#[rstest]
#[case::absent(None)]
#[case::null(Some("null"))]
#[case::empty_object(Some(r#"{"billed_units":{}}"#))]
#[case::billed_null(Some(r#"{"billed_units":null}"#))]
#[case::image_tokens_alone(Some(r#"{"billed_units":{"image_tokens":1000}}"#))]
fn a_missing_billed_pair_counts_the_input_locally(#[case] source: Option<&str>) {
    let fixture = fixture(None, &["hello", "world!"]);
    let meta = source.map(json);
    let usage = calculate_usage(&texts(&["hello", "world!"]), &fixture, meta.as_ref()).unwrap();
    assert_eq!(
        (usage.prompt_tokens, usage.prompt_tokens_details),
        (11, None)
    );
    assert_eq!(
        *fixture.encoded.lock().unwrap(),
        texts(&["hello", "world!"])
    );
}

#[test]
fn billed_overflow_and_a_non_integer_billed_unit_are_errors() {
    let fixture = fixture(None, &["hello"]);
    let overflow = params(r#"{"billed_units":{"input_tokens":9223372036854775807,"images":1}}"#);
    assert!(matches!(
        calculate_usage(&texts(&["hello"]), &fixture, Some(&Json::Object(overflow))),
        Err(CohereEmbeddingError::UsageOverflow)
    ));
    let wrong = params(r#"{"billed_units":{"input_tokens":"five"}}"#);
    assert!(matches!(
        calculate_usage(&texts(&["hello"]), &fixture, Some(&Json::Object(wrong))),
        Err(CohereEmbeddingError::Json(_))
    ));
}

type Decoded = Result<CohereEmbeddingResponse, CohereEmbeddingError>;

fn decode(body: &str, input: &[&str], fail_on: Option<&str>) -> (Decoded, Fixture, Fixture) {
    let request = transform_request("m", &texts(input), &Object::new());
    let encoding = fixture(fail_on, input);
    let mut logging = fixture(None, input);
    let reply = DecodeInput {
        body: body.as_bytes(),
        model: "request-model",
        api_key: Some(&SecretString::from("sk-test")),
        request_data: &request,
        encoding: &encoding,
    };
    let decoded = transform_embedding_response(reply, &mut logging);
    (decoded, encoding, logging)
}

#[test]
fn groups_flatten_in_answer_order_with_indices_restarting() {
    let (decoded, _, logging) = decode(
        r#"{"embeddings":{"float":[[0.5],[0.25]],"int8":[[1],[2]]},"meta":{"billed_units":{"input_tokens":4,"images":2}}}"#,
        &["first", "second"],
        None,
    );
    let response = decoded.unwrap();
    assert_eq!(
        (response.object.as_str(), response.model.as_str()),
        ("list", "request-model")
    );
    let rows: Vec<(usize, Json)> = response
        .data
        .iter()
        .map(|row| (row.index, row.embedding.clone()))
        .collect();
    assert_eq!(
        rows,
        vec![
            (0, json("[0.5]")),
            (1, json("[0.25]")),
            (0, json("[1]")),
            (1, json("[2]"))
        ]
    );
    assert_eq!(response.usage.prompt_tokens, 6);
    assert_eq!(logging.calls.len(), 1);
    assert_eq!(logging.calls[0]["api_key"], "sk-test");
    assert_eq!(
        logging.calls[0]["input"],
        serde_json::json!(["first", "second"])
    );
    assert_eq!(
        logging.calls[0]["complete_input_dict"]["input_type"],
        "search_document"
    );
}

#[test]
fn post_call_precedes_a_missing_embeddings_object_and_invalid_json_records_none() {
    let (missing, _, missing_log) = decode(r#"{"meta":{}}"#, &["hello"], None);
    assert!(matches!(missing, Err(CohereEmbeddingError::Json(_))));
    assert_eq!(missing_log.calls.len(), 1);
    let (invalid, _, invalid_log) = decode("{not json", &["hello"], None);
    assert!(matches!(invalid, Err(CohereEmbeddingError::Json(_))));
    assert!(invalid_log.calls.is_empty());
}

#[test]
fn the_preliminary_pass_encodes_every_input_even_with_billed_units() {
    let billed = r#"{"embeddings":{"float":[[0.5]]},"meta":{"billed_units":{"input_tokens":9}}}"#;
    let (decoded, encoding, _) = decode(billed, &["first", "second"], None);
    assert_eq!(decoded.unwrap().usage.prompt_tokens, 9);
    assert_eq!(
        *encoding.encoded.lock().unwrap(),
        texts(&["first", "second"])
    );
    let (failed, failed_encoding, failed_log) = decode(billed, &["first", "boom"], Some("boom"));
    assert!(matches!(failed, Err(CohereEmbeddingError::Encoding(_))));
    assert_eq!(
        *failed_encoding.encoded.lock().unwrap(),
        texts(&["first", "boom"])
    );
    assert_eq!(failed_log.calls.len(), 1);
}

#[test]
fn the_fallback_pass_encodes_twice_and_the_transform_reads_logged_input() {
    let (counted, encoding, _) = decode(
        r#"{"embeddings":{"float":[[0.5]]}}"#,
        &["first", "second"],
        None,
    );
    assert_eq!(counted.unwrap().usage.prompt_tokens, 11);
    assert_eq!(
        *encoding.encoded.lock().unwrap(),
        texts(&["first", "second", "first", "second"])
    );
    let request = transform_request("m", &texts(&["from-details"]), &Object::new());
    let detail_encoding = fixture(None, &["from-details"]);
    let mut logging = fixture(None, &["from-details"]);
    let reply = DecodeInput {
        body: br#"{"embeddings":{"float":[[1.5]]}}"#,
        model: "request-model",
        api_key: None,
        request_data: &request,
        encoding: &detail_encoding,
    };
    let transformed = transform_embedding_response(reply, &mut logging).unwrap();
    assert_eq!(transformed.usage.prompt_tokens, 12);
    assert_eq!(
        logging.calls[0]["input"],
        serde_json::json!(["from-details"])
    );
}

#[rstest]
fn replays_the_cohere_embeddings_corpus(#[files("fuzz/corpus/cohere_embeddings/*")] path: PathBuf) {
    let body = fs::read(path).unwrap();
    let request = transform_request("m", &["hello".to_owned()], &Object::new());
    let encoding = fixture(None, &["hello"]);
    let mut logging = fixture(None, &["hello"]);
    let reply = DecodeInput {
        body: &body,
        model: "m",
        api_key: None,
        request_data: &request,
        encoding: &encoding,
    };
    let _unused = transform_embedding_response(reply, &mut logging);
}

#[test]
fn the_real_engine_counts_the_encoded_tokens() {
    let data = assets::bundled(Encoding::P50kBase).unwrap();
    let engine = Engine::Tiktoken(Arc::new(
        TiktokenEngine::from_bpe_file(Encoding::P50kBase, &data).unwrap(),
    ));
    assert_eq!(engine.count_tokens("hello world").unwrap(), 2);
    assert_eq!(engine.count_tokens("").unwrap(), 0);
}
