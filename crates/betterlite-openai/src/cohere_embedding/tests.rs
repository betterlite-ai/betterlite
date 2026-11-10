#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::error::EngineError;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::request::{
    CohereInputType, CohereTruncate, get_complete_url, map_openai_params,
    transform_embedding_request, transform_request, validate_environment,
};
use super::response::{
    CohereEmbeddingResponse, DecodeInput, TransformInput, decode_cohere_embeddings,
    transform_embedding_response,
};
use super::usage::calculate_usage;
use super::{
    CohereEmbeddingError, DEFAULT_COMPLETE_URL, DEFAULT_EMBEDDING_INPUT_TYPE, EmbeddingLogging,
    ModelCallDetails, PostCall, SUPPORTED_OPENAI_PARAMS, TextEncoding,
};

struct Fixture {
    encoded: Mutex<Vec<String>>,
    fail_on: Option<String>,
    details: ModelCallDetails,
    calls: Vec<Json>,
}

impl TextEncoding for Fixture {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        self.encoded.lock().unwrap().push(text.to_owned());
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
    fn model_call_details(&self) -> &ModelCallDetails {
        &self.details
    }

    fn post_call(&mut self, call: PostCall<'_>) {
        self.calls.push(serde_json::json!({
            "input": call.input,
            "api_key": call.api_key.map(|key| key.expose_secret().to_owned()),
            "complete_input_dict": call.complete_input_dict,
            "original_response": call.original_response,
        }));
    }
}

fn fixture(fail_on: Option<&str>, input: &[&str]) -> Fixture {
    Fixture {
        encoded: Mutex::new(Vec::new()),
        fail_on: fail_on.map(str::to_owned),
        details: ModelCallDetails {
            input: texts(input),
        },
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
    let request =
        transform_request("m", &serde_json::from_str(source).unwrap(), &Object::new()).unwrap();
    let value = serde_json::to_value(&request).unwrap();
    assert_eq!(value["model"], "m");
    assert_eq!(value["input_type"], input_type);
    assert_eq!(value[key], serde_json::from_str::<Json>(source).unwrap());
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
    assert_eq!(wrapped.texts, Some(texts(&["hello"])));
    let empty = transform_request("m", &[], &Object::new()).unwrap();
    assert_eq!(empty.texts, Some(Vec::new()));
    assert_eq!(empty.input_type, CohereInputType::SearchDocument);
    assert!(matches!(
        transform_embedding_request("m", &serde_json::from_str("[]").unwrap(), &Object::new()),
        Err(CohereEmbeddingError::EmptyInput)
    ));
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
    let request = transform_request(
        "m",
        &texts(&["hello"]),
        &params(r#"{"model":"other","texts":["over"],"images":["pic"],"input_type":"clustering","truncate":"START","embedding_types":["int8"],"output_dimension":64}"#),
    )
    .unwrap();
    assert_eq!(request.model, "other");
    assert_eq!(request.texts, Some(texts(&["over"])));
    assert_eq!(request.images, Some(texts(&["pic"])));
    assert_eq!(request.input_type, CohereInputType::Clustering);
    assert_eq!(request.truncate, Some(CohereTruncate::Start));
    assert_eq!(
        request.embedding_types,
        Some(vec![Json::Text(String::from("int8"))])
    );
    assert_eq!(request.output_dimension, Some(64));
    assert_eq!(
        transform_request("m", &texts(&["hello"]), &params("{}"))
            .unwrap()
            .model,
        "m"
    );
}

#[rstest]
#[case::unknown(r#"{"unknown":1}"#)]
#[case::model(r#"{"model":1}"#)]
#[case::texts(r#"{"texts":"one"}"#)]
#[case::input_type(r#"{"input_type":"other"}"#)]
#[case::truncate(r#"{"truncate":"MIDDLE"}"#)]
#[case::dimension(r#"{"output_dimension":"big"}"#)]
fn a_parameter_the_pinned_types_reject_is_an_error(#[case] source: &str) {
    let refused = transform_request("m", &texts(&["hello"]), &params(source));
    assert!(matches!(refused, Err(CohereEmbeddingError::Json { .. })));
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
    assert_eq!(
        serde_json::to_value(CohereInputType::SearchDocument).unwrap(),
        serde_json::json!(DEFAULT_EMBEDDING_INPUT_TYPE)
    );
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
        crate::openai_base_config::Headers::from([(
            String::from("Content-Type"),
            String::from("text/plain"),
        )]),
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
fn a_missing_billed_pair_counts_the_input_locally(#[case] meta: Option<&str>) {
    let fixture = fixture(None, &["hello", "world!"]);
    let meta = meta.map(|source| Json::Object(params(source)));
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
        Err(CohereEmbeddingError::Field {
            name: "meta.billed_units.input_tokens",
            ..
        })
    ));
}

type Decoded = Result<CohereEmbeddingResponse, CohereEmbeddingError>;

fn decode(body: &str, input: &[&str], fail_on: Option<&str>) -> (Decoded, Fixture, Fixture) {
    let request = transform_request("m", &texts(input), &Object::new()).unwrap();
    let encoding = fixture(fail_on, input);
    let mut logging = fixture(None, input);
    let decoded = decode_cohere_embeddings(
        DecodeInput {
            body: body.as_bytes(),
            model: "request-model",
            input: &texts(input),
            api_key: Some(&SecretString::from("sk-test")),
            request_data: &request,
            encoding: &encoding,
        },
        &mut logging,
    );
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
    let (missing, _, logging) = decode(r#"{"meta":{}}"#, &["hello"], None);
    assert!(matches!(
        missing,
        Err(CohereEmbeddingError::Field {
            name: "embeddings",
            ..
        })
    ));
    assert_eq!(logging.calls.len(), 1);
    let (invalid, _, logging) = decode("{not json", &["hello"], None);
    assert!(matches!(invalid, Err(CohereEmbeddingError::Json { .. })));
    assert!(logging.calls.is_empty());
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
    let (failed, encoding, logging) = decode(billed, &["first", "boom"], Some("boom"));
    assert!(matches!(failed, Err(CohereEmbeddingError::Encoding { .. })));
    assert_eq!(*encoding.encoded.lock().unwrap(), texts(&["first", "boom"]));
    assert_eq!(logging.calls.len(), 1);
}

#[test]
fn the_fallback_pass_encodes_twice_and_the_transform_reads_logged_input() {
    let (decoded, encoding, _) = decode(
        r#"{"embeddings":{"float":[[0.5]]}}"#,
        &["first", "second"],
        None,
    );
    assert_eq!(decoded.unwrap().usage.prompt_tokens, 11);
    assert_eq!(
        *encoding.encoded.lock().unwrap(),
        texts(&["first", "second", "first", "second"])
    );
    let request = transform_request("m", &texts(&["from-details"]), &Object::new()).unwrap();
    let encoding = fixture(None, &["from-details"]);
    let mut logging = fixture(None, &["from-details"]);
    let decoded = transform_embedding_response(
        TransformInput {
            body: br#"{"embeddings":{"float":[[1.5]]}}"#,
            model: "request-model",
            api_key: None,
            request_data: &request,
            encoding: &encoding,
        },
        &mut logging,
    )
    .unwrap();
    assert_eq!(decoded.usage.prompt_tokens, 12);
    assert_eq!(
        logging.calls[0]["input"],
        serde_json::json!(["from-details"])
    );
}

#[rstest]
fn replays_the_cohere_embeddings_corpus(#[files("fuzz/corpus/cohere_embeddings/*")] path: PathBuf) {
    let body = fs::read(path).unwrap();
    let request = transform_request("m", &["hello".to_owned()], &Object::new()).unwrap();
    let encoding = fixture(None, &["hello"]);
    let mut logging = fixture(None, &["hello"]);
    let _unused = decode_cohere_embeddings(
        DecodeInput {
            body: &body,
            model: "m",
            input: &texts(&["hello"]),
            api_key: None,
            request_data: &request,
            encoding: &encoding,
        },
        &mut logging,
    );
}
