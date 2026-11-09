#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::str;
use std::sync::Mutex;

use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::embedding::EmbeddingVector;
use betterlite_prompts::builtin::Field;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::Hub;
use betterlite_prompts::factory::hub::builtin_entries;
use chrono::DateTime;
use chrono::Utc;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::config::SUPPORTED_OPENAI_PARAMS;
use super::config::TritonEndpoint;
use super::config::complete_url;
use super::config::map_openai_params;
use super::config::validate_environment;
use super::datatype::TritonDatatype;
use super::embedding::EmbeddingConfig as _;
use super::embedding::EmbeddingRequest;
use super::embedding::TextTokenCounter;
use super::embedding::TritonEmbeddingConfig;
use super::embedding::build_embedding_usage;
use super::embedding::split_embedding_by_shape;
use super::embedding::transform_embedding_request;
use super::embedding::transform_embedding_response;
use super::error::TritonError;
use super::generate::TritonGenerateConfig;
use super::generate::max_tokens_default;
use super::generate::transform_response as generate_response;
use super::infer::transform_request as infer_request;
use super::infer::transform_response as infer_response;
use super::request::TransformInput;
use super::router::TritonReply;
use super::router::TritonRequest;
use super::router::transform_request as route_request;
use super::router::transform_response as route_response;
use super::stream::decode_stream_line;

const LLAMA2_PROMPT: &str = "<s>[INST] hi [/INST]\n";

fn factory() -> PromptFactory {
    PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Unreachable,
    }
}

fn clock() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn config(prompts: &PromptFactory) -> TritonGenerateConfig<'_> {
    TritonGenerateConfig {
        prompts,
        now: clock(),
        max_tokens_default: 2000,
    }
}

fn options(json: &str) -> IndexMap<String, Field> {
    serde_json::from_str(json).unwrap()
}

fn messages(json: &str) -> Vec<Message> {
    serde_json::from_str(json).unwrap()
}

fn user_message() -> Vec<Message> {
    messages(r#"[{"role":"user","content":"hi"}]"#)
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn input<'request>(
    model: &'request str,
    messages: &'request [Message],
    optional_params: &'request IndexMap<String, Field>,
) -> TransformInput<'request> {
    TransformInput {
        model,
        messages,
        optional_params,
    }
}

struct FixedCounter(Option<usize>);

impl TextTokenCounter for FixedCounter {
    fn count_text(&self, _model: &str, _text: &str) -> Option<usize> {
        self.0
    }
}

struct RecordingCounter<'seen>(&'seen Mutex<Vec<String>>);

impl TextTokenCounter for RecordingCounter<'_> {
    fn count_text(&self, _model: &str, text: &str) -> Option<usize> {
        self.0.lock().unwrap().push(text.to_owned());
        Some(1)
    }
}

struct WordsCounter;

impl TextTokenCounter for WordsCounter {
    fn count_text(&self, _model: &str, text: &str) -> Option<usize> {
        Some(text.split_whitespace().count())
    }
}

fn embedding_request(json: &str) -> EmbeddingRequest {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::generate("https://host/generate", Some(TritonEndpoint::Generate))]
#[case::infer("https://host/infer", Some(TritonEndpoint::Infer))]
#[case::nested_path("https://host/v1/models/m/infer", Some(TritonEndpoint::Infer))]
#[case::stream_suffix("https://host/generate_stream", None)]
#[case::other_path("https://host/v2/models/m", None)]
#[case::empty("", None)]
fn endpoint_is_chosen_by_the_api_base(
    #[case] api_base: &str,
    #[case] expected: Option<TritonEndpoint>,
) {
    if let Some(endpoint) = expected {
        assert_eq!(TritonEndpoint::from_api_base(api_base).unwrap(), endpoint);
    } else {
        let error = TritonEndpoint::from_api_base(api_base).unwrap_err();
        assert!(matches!(error, TritonError::InvalidApiBase { .. }));
        assert!(error.to_string().contains("Invalid Triton API base"));
    }
}

#[rstest]
#[case::generate_stream(
    Some("https://host/generate"),
    true,
    Some("https://host/generate_stream")
)]
#[case::generate_plain(Some("https://host/generate"), false, Some("https://host/generate"))]
#[case::infer_stream(Some("https://host/infer"), true, Some("https://host/infer"))]
#[case::missing_base(None, false, None)]
#[case::unknown_base(Some("https://host/other"), true, None)]
fn complete_url_follows_the_endpoint(
    #[case] api_base: Option<&str>,
    #[case] stream: bool,
    #[case] expected: Option<&str>,
) {
    assert_eq!(complete_url(api_base, stream).ok().as_deref(), expected);
}

#[test]
fn complete_url_rejects_a_missing_api_base() {
    let error = complete_url(None, false).unwrap_err();
    assert!(matches!(error, TritonError::MissingApiBase));
    assert_eq!(error.status(), None);
}

#[test]
fn supported_params_are_the_two_max_token_names() {
    assert_eq!(
        SUPPORTED_OPENAI_PARAMS,
        ["max_tokens", "max_completion_tokens"]
    );
}

#[test]
fn map_openai_params_copies_only_the_two_names() {
    let non_default = options(r#"{"max_tokens":5,"temperature":1,"max_completion_tokens":7}"#);
    let mut optional = options(r#"{"kept":true}"#);
    map_openai_params(&non_default, &mut optional);
    let names: Vec<&str> = optional.keys().map(String::as_str).collect();
    assert_eq!(names, ["kept", "max_tokens", "max_completion_tokens"]);
    assert_eq!(
        optional.get("max_tokens"),
        Some(&Field::Number(Number::from(5_i64)))
    );
    assert_eq!(
        optional.get("max_completion_tokens"),
        Some(&Field::Number(Number::from(7_i64)))
    );
}

#[test]
fn validate_environment_sets_the_json_content_type() {
    let headers = validate_environment();
    assert_eq!(headers.len(), 1);
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
}

#[tokio::test]
async fn generate_request_renders_the_prompt() {
    let prompts = factory();
    let messages = user_message();
    let optional = options("{}");
    let body = config(&prompts)
        .transform_request(&input("meta-llama/llama-2-7b-chat", &messages, &optional))
        .await
        .unwrap();
    assert_eq!(body.text_input.as_deref(), Some(LLAMA2_PROMPT));
    assert_eq!(body.parameters, options(r#"{"max_tokens":2000}"#));
    assert!(!body.stream);
}

#[rstest]
#[case::default("{}", r#"{"max_tokens":2000}"#, false)]
#[case::stream_true(r#"{"stream":true}"#, r#"{"max_tokens":2000}"#, true)]
#[case::stream_false(r#"{"stream":false}"#, r#"{"max_tokens":2000}"#, false)]
#[case::stream_null(r#"{"stream":null}"#, r#"{"max_tokens":2000}"#, false)]
#[case::max_tokens(r#"{"max_tokens":5}"#, r#"{"max_tokens":5}"#, false)]
#[case::stream_and_max_tokens(r#"{"stream":true,"max_tokens":5}"#, r#"{"max_tokens":5}"#, true)]
#[case::other_params(
    r#"{"max_completion_tokens":9,"temperature":0.5}"#,
    r#"{"max_tokens":2000,"max_completion_tokens":9,"temperature":0.5}"#,
    false
)]
#[tokio::test]
async fn generate_request_carries_the_options_and_stream_flag(
    #[case] given: &str,
    #[case] expected_parameters: &str,
    #[case] expected_stream: bool,
) {
    let prompts = factory();
    let given_messages = user_message();
    let optional = options(given);
    let body = config(&prompts)
        .transform_request(&input(
            "meta-llama/llama-2-7b-chat",
            &given_messages,
            &optional,
        ))
        .await
        .unwrap();
    assert_eq!(body.parameters, options(expected_parameters));
    assert_eq!(body.stream, expected_stream);
}

#[tokio::test]
async fn generate_request_without_a_prompt_sends_a_null_text_input() {
    let prompts = factory();
    let given = user_message();
    let optional = options("{}");
    let body = config(&prompts)
        .transform_request(&input("mosaicml/mpt-7b", &given, &optional))
        .await
        .unwrap();
    assert_eq!(body.text_input, None);
}

#[rstest]
#[case::null_max_tokens(r#"{"max_tokens":null}"#)]
#[case::float_max_tokens(r#"{"max_tokens":5.5}"#)]
#[case::text_max_tokens(r#"{"max_tokens":"5"}"#)]
#[case::negative_float(r#"{"max_tokens":-1.0}"#)]
#[tokio::test]
async fn generate_request_rejects_a_max_tokens_that_is_not_an_integer(#[case] given: &str) {
    let prompts = factory();
    let given_messages = user_message();
    let optional = options(given);
    let error = config(&prompts)
        .transform_request(&input(
            "meta-llama/llama-2-7b-chat",
            &given_messages,
            &optional,
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        TritonError::NotAnInteger { name: "max_tokens" }
    ));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::text_stream(r#"{"stream":"yes"}"#)]
#[case::number_stream(r#"{"stream":1}"#)]
#[tokio::test]
async fn generate_request_rejects_a_stream_that_is_not_a_boolean(#[case] given: &str) {
    let prompts = factory();
    let given_messages = user_message();
    let optional = options(given);
    let error = config(&prompts)
        .transform_request(&input(
            "meta-llama/llama-2-7b-chat",
            &given_messages,
            &optional,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, TritonError::NotABoolean { name: "stream" }));
}

#[rstest]
#[case::absent(&[], 2000)]
#[case::configured(&[("DEFAULT_MAX_TOKENS_FOR_TRITON", "123")], 123)]
#[case::padded(&[("DEFAULT_MAX_TOKENS_FOR_TRITON", " 512 ")], 512)]
#[case::empty(&[("DEFAULT_MAX_TOKENS_FOR_TRITON", "")], 2000)]
fn max_tokens_default_reads_the_environment(#[case] pairs: &[(&str, &str)], #[case] expected: i64) {
    assert_eq!(max_tokens_default(&environment(pairs)).unwrap(), expected);
}

#[test]
fn max_tokens_default_rejects_a_value_that_is_not_a_number() {
    let error =
        max_tokens_default(&environment(&[("DEFAULT_MAX_TOKENS_FOR_TRITON", "many")])).unwrap_err();
    assert!(matches!(error, TritonError::Environment { .. }));
    assert!(error.to_string().contains("DEFAULT_MAX_TOKENS_FOR_TRITON"));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::text(br#"{"text_output":"hello"}"#, Some("hello"))]
#[case::null(br#"{"text_output":null}"#, None)]
#[case::extra_fields(br#"{"text_output":"hi","other":{"nested":1}}"#, Some("hi"))]
fn generate_response_reads_the_text_output(#[case] body: &[u8], #[case] expected: Option<&str>) {
    let reply = generate_response(200, body).unwrap();
    assert_eq!(reply.text.as_deref(), expected);
}

#[rstest]
#[case::missing_field(br#"{"other":1}"#)]
#[case::wrong_type(br#"{"text_output":5}"#)]
#[case::scalar(b"5")]
#[case::array(b"[]")]
#[case::array_of_the_field(br#"["hi"]"#)]
fn generate_response_rejects_a_body_of_the_wrong_shape(#[case] body: &[u8]) {
    let error = generate_response(200, body).unwrap_err();
    assert!(matches!(error, TritonError::ReplyShape { .. }));
    assert_eq!(error.status(), None);
}

#[test]
fn generate_response_rejects_unreadable_json_with_the_response_status() {
    let error = generate_response(503, b"{").unwrap_err();
    assert!(matches!(
        error,
        TritonError::Api {
            status_code: 503,
            ..
        }
    ));
    assert_eq!(error.status(), Some(503));
    assert!(error.to_string().contains('{'));
}

#[test]
fn api_error_carries_the_status_message_and_headers() {
    let mut headers = http::HeaderMap::new();
    headers.insert("x-request-id", "r-1".parse().unwrap());
    let error = TritonError::Api {
        status_code: 429,
        message: String::from("upstream failed"),
        headers,
    };
    assert_eq!(error.status(), Some(429));
    assert!(error.to_string().contains("upstream failed"));
    let TritonError::Api {
        headers: carried, ..
    } = error
    else {
        panic!("expected the API error");
    };
    assert_eq!(carried.get("x-request-id").unwrap(), "r-1");
}

#[test]
fn reply_decoder_rejects_non_utf8_with_the_response_status() {
    let error = generate_response(400, &[0xff, 0xfe]).unwrap_err();
    assert_eq!(error.status(), Some(400));
    assert!(error.to_string().contains("2 bytes that are not UTF-8"));
}

#[test]
fn infer_request_names_its_inputs_in_order() {
    let optional =
        options(r#"{"max_tokens":7,"temperature":0.5,"flag":true,"stream":true,"max_retries":2}"#);
    let given = messages(r#"[{"role":"user","content":"hello"}]"#);
    let body = infer_request(&input("model", &given, &optional)).unwrap();
    let names: Vec<&str> = body
        .inputs
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["text_input", "max_tokens", "temperature", "flag"]);
    assert_eq!(
        body.inputs[0].data,
        vec![Field::Text(String::from("hello"))]
    );
    assert_eq!(body.inputs[1].shape, [1]);
    assert_eq!(body.inputs[1].datatype, TritonDatatype::Int32);
    assert_eq!(body.inputs[2].datatype, TritonDatatype::Fp32);
    assert_eq!(body.inputs[3].datatype, TritonDatatype::Int32);
}

#[test]
fn infer_request_appends_the_default_max_tokens() {
    let optional = options("{}");
    let given = user_message();
    let body = infer_request(&input("model", &given, &optional)).unwrap();
    assert_eq!(body.inputs.len(), 2);
    assert_eq!(body.inputs[1].name, "max_tokens");
    assert_eq!(body.inputs[1].datatype, TritonDatatype::Int32);
    assert_eq!(
        body.inputs[1].data,
        vec![Field::Number(Number::from(20_i64))]
    );
}

#[test]
fn infer_request_maps_values_to_triton_datatypes() {
    let optional = options(
        r#"{"count":3,"ratio":1.5,"label":"x","empty":null,"items":[1],"flag":true,"big":18446744073709551615,"negative":-3}"#,
    );
    let given = user_message();
    let body = infer_request(&input("model", &given, &optional)).unwrap();
    let datatypes: Vec<&str> = body
        .inputs
        .iter()
        .map(|entry| match entry.datatype {
            TritonDatatype::Int32 => "INT32",
            TritonDatatype::Fp32 => "FP32",
            TritonDatatype::Bytes => "BYTES",
        })
        .collect();
    assert_eq!(
        datatypes,
        [
            "BYTES", "INT32", "FP32", "BYTES", "BYTES", "BYTES", "INT32", "INT32", "INT32", "INT32"
        ]
    );
}

#[rstest]
#[case::first_of_several(
    r#"[{"role":"user","content":"first"},{"role":"assistant","content":"second"}]"#,
    r#""first""#
)]
#[case::null(r#"[{"role":"assistant","content":null}]"#, "null")]
#[case::parts(
    r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#,
    r#"[{"type":"text","text":"hi"}]"#
)]
fn infer_request_sends_the_first_message_content(#[case] json: &str, #[case] expected: &str) {
    let optional = options("{}");
    let given = messages(json);
    let body = infer_request(&input("model", &given, &optional)).unwrap();
    let field: Field = serde_json::from_str(expected).unwrap();
    assert_eq!(body.inputs[0].data, vec![field]);
}

#[test]
fn infer_request_rejects_an_empty_message_list() {
    let optional = options("{}");
    let given: Vec<Message> = Vec::new();
    let error = infer_request(&input("model", &given, &optional)).unwrap_err();
    assert!(matches!(error, TritonError::NoMessages));
}

#[rstest]
#[case::string(br#"{"outputs":[{"data":"hello"}]}"#, Some("hello"))]
#[case::list(br#"{"outputs":[{"data":["he","llo"]}]}"#, Some("hello"))]
#[case::empty_list(br#"{"outputs":[{"data":[]}]}"#, Some(""))]
#[case::null(br#"{"outputs":[{"data":null}]}"#, None)]
#[case::later_outputs_unread(br#"{"outputs":[{"data":"a"},["x"]]}"#, Some("a"))]
fn infer_response_reads_the_data(#[case] body: &[u8], #[case] expected: Option<&str>) {
    let reply = infer_response(200, body).unwrap();
    assert_eq!(reply.text.as_deref(), expected);
}

#[rstest]
#[case::no_outputs(br#"{"outputs":[]}"#)]
#[case::missing_outputs(br#"{"other":1}"#)]
#[case::missing_data(br#"{"outputs":[{"shape":[1]}]}"#)]
#[case::number_data(br#"{"outputs":[{"data":5}]}"#)]
#[case::mixed_list(br#"{"outputs":[{"data":["a",1]}]}"#)]
#[case::array_body(br#"[[{"data":"hi"}]]"#)]
#[case::array_output(br#"{"outputs":[["hi"]]}"#)]
fn infer_response_rejects_a_body_it_cannot_read(#[case] body: &[u8]) {
    infer_response(200, body).unwrap_err();
}

#[test]
fn infer_response_reports_an_empty_output_list() {
    let error = infer_response(200, br#"{"outputs":[]}"#).unwrap_err();
    assert!(matches!(error, TritonError::NoOutputs));
}

#[test]
fn embedding_request_sends_a_text_as_one_input() {
    let request = transform_embedding_request(EmbeddingInput::Text(String::from("hello")));
    assert_eq!(request.inputs.len(), 1);
    let entry = &request.inputs[0];
    assert_eq!(entry.name, "input_text");
    assert_eq!(entry.shape, [5]);
    assert_eq!(entry.datatype, TritonDatatype::Bytes);
    assert_eq!(entry.data, EmbeddingInput::Text(String::from("hello")));
}

#[rstest]
#[case::texts(EmbeddingInput::Texts(vec![String::from("hello"), String::from("world")]), 2)]
#[case::tokens(EmbeddingInput::Tokens(vec![1, 2, 3]), 3)]
#[case::token_lists(EmbeddingInput::TokenLists(vec![vec![1, 2], vec![3]]), 2)]
fn embedding_request_shape_is_the_input_length(
    #[case] given: EmbeddingInput,
    #[case] expected: usize,
) {
    let request = transform_embedding_request(given.clone());
    assert_eq!(request.inputs[0].shape, [expected]);
    assert_eq!(request.inputs[0].data, given);
}

#[test]
fn embedding_response_splits_the_outputs_by_shape() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let body =
        br#"{"model_name":"trtllm","outputs":[{"shape":[2,3],"data":[1.5,2.5,3.5,4.5,5.5,6.5]}]}"#;
    let reply = transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap();
    assert_eq!(reply.model, "trtllm");
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(
        reply.data[0].embedding,
        EmbeddingVector::Floats(vec![
            Number::from_f64(1.5).unwrap(),
            Number::from_f64(2.5).unwrap(),
            Number::from_f64(3.5).unwrap(),
        ])
    );
}

#[test]
fn embedding_response_rejects_a_shape_that_is_not_a_list_of_integers() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let body = br#"{"outputs":[{"shape":"2x3","data":[1.5,2.5,3.5,4.5,5.5,6.5]}]}"#;
    let error =
        transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap_err();
    assert!(matches!(error, TritonError::ReplyShape { .. }));
    assert_eq!(error.status(), None);
}

#[test]
fn embedding_response_defaults_the_model_to_the_literal_none() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let body = br#"{"outputs":[{"shape":[1,1],"data":[1.5]}]}"#;
    let reply = transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap();
    assert_eq!(reply.model, "None");
}

#[test]
fn embedding_response_counts_the_request_texts() {
    let request = embedding_request(
        r#"{"inputs":[{"name":"input_text","shape":[2],"datatype":"BYTES","data":["hello world",""]}]}"#,
    );
    let body = br#"{"outputs":[{"shape":[1,1],"data":[1.5]}]}"#;
    let reply = transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap();
    assert_eq!(reply.usage.prompt_tokens, 2);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 2);
}

#[rstest]
#[case::injected_counter(
    r#"{"inputs":[{"name":"input_text","shape":[1],"datatype":"BYTES","data":["hello"]}]}"#,
    &FixedCounter(Some(7)),
    7
)]
#[case::skips_an_empty_text(
    r#"{"inputs":[{"name":"input_text","shape":[2],"datatype":"BYTES","data":["","x"]}]}"#,
    &FixedCounter(Some(5)),
    5
)]
#[case::whitespace_fallback(
    r#"{"inputs":[{"name":"input_text","shape":[1],"datatype":"BYTES","data":["a b  c"]}]}"#,
    &FixedCounter(None),
    3
)]
#[case::first_named_input(
    r#"{"inputs":[{"name":"other","shape":[1],"datatype":"BYTES","data":["ignored ignored"]},{"name":"input_text","shape":[1],"datatype":"BYTES","data":["one two"]},{"name":"input_text","shape":[1],"datatype":"BYTES","data":["three"]}]}"#,
    &WordsCounter,
    2
)]
#[case::no_named_input(r#"{"inputs":[]}"#, &WordsCounter, 0)]
#[case::text_data_entry(
    r#"{"inputs":[{"name":"input_text","shape":[5],"datatype":"BYTES","data":"hello"}]}"#,
    &WordsCounter,
    0
)]
fn embedding_usage_counts_the_named_input(
    #[case] json: &str,
    #[case] counter: &dyn TextTokenCounter,
    #[case] expected: u32,
) {
    let usage = build_embedding_usage("model", &embedding_request(json), counter).unwrap();
    assert_eq!(usage.prompt_tokens, expected);
    assert_eq!(usage.total_tokens, expected);
}

#[rstest]
#[case::token_ids(
    r#"{"inputs":[{"name":"input_text","shape":[3],"datatype":"BYTES","data":[1,22,333]}]}"#,
    &["1", "22", "333"]
)]
#[case::token_lists(
    r#"{"inputs":[{"name":"input_text","shape":[2],"datatype":"BYTES","data":[[1,2],[3]]}]}"#,
    &["[1, 2]", "[3]"]
)]
fn embedding_usage_counts_the_text_form_of_token_input(
    #[case] json: &str,
    #[case] counted: &[&str],
) {
    let seen = Mutex::new(Vec::new());
    let usage =
        build_embedding_usage("model", &embedding_request(json), &RecordingCounter(&seen)).unwrap();
    assert_eq!(usage.prompt_tokens, u32::try_from(counted.len()).unwrap());
    assert_eq!(*seen.lock().unwrap(), counted);
}

#[test]
fn embedding_config_sends_no_headers_and_keeps_the_params() {
    let params = options(r#"{"kept":true}"#);
    assert!(TritonEmbeddingConfig.validate_environment().is_empty());
    assert_eq!(
        TritonEmbeddingConfig.map_openai_params(params.clone()),
        params
    );
}

#[rstest]
#[case::two_by_three(
    vec![1.5_f64, 2.5_f64, 3.5_f64, 4.5_f64, 5.5_f64, 6.5_f64],
    vec![2_i64, 3],
    vec![
        vec![1.5_f64, 2.5_f64, 3.5_f64],
        vec![4.5_f64, 5.5_f64, 6.5_f64]
    ]
)]
#[case::short_data(
    vec![1.5_f64, 2.5_f64],
    vec![2_i64, 3],
    vec![vec![1.5_f64, 2.5_f64], vec![]]
)]
#[case::long_data(
    vec![1.5_f64, 2.5_f64, 3.5_f64, 4.5_f64],
    vec![1_i64, 3],
    vec![vec![1.5_f64, 2.5_f64, 3.5_f64]]
)]
#[case::zero_count(vec![], vec![0_i64, 3], vec![])]
#[case::zero_width(vec![1.5_f64, 2.5_f64], vec![2_i64, 0], vec![vec![], vec![]])]
fn split_embedding_by_shape_keeps_the_python_slices(
    #[case] data: Vec<f64>,
    #[case] shape: Vec<i64>,
    #[case] expected: Vec<Vec<f64>>,
) {
    assert_eq!(split_embedding_by_shape(&data, &shape).unwrap(), expected);
}

#[rstest]
#[case::one_dimension(vec![1_i64])]
#[case::three_dimensions(vec![1_i64, 2, 1])]
#[case::no_dimensions(vec![])]
fn split_embedding_by_shape_rejects_another_length(#[case] shape: Vec<i64>) {
    let error = split_embedding_by_shape(&[1.5_f64], &shape).unwrap_err();
    assert!(matches!(error, TritonError::ShapeLength));
    assert!(error.to_string().contains("Shape must be of length 2."));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::negative_count(vec![1.5_f64], vec![-1_i64, 1])]
#[case::negative_width(vec![1.5_f64], vec![1_i64, -1])]
fn split_embedding_by_shape_rejects_negative_values(
    #[case] data: Vec<f64>,
    #[case] shape: Vec<i64>,
) {
    assert!(matches!(
        split_embedding_by_shape(&data, &shape),
        Err(TritonError::ShapeValue { .. })
    ));
}

#[test]
fn split_embedding_by_shape_bounds_the_chunk_count() {
    let error = split_embedding_by_shape(&[1.5_f64], &[3_i64, 1]).unwrap_err();
    assert!(matches!(
        error,
        TritonError::ShapeExceedsData {
            count: 3,
            elements: 1
        }
    ));
}

#[test]
fn embedding_response_splits_every_output_from_index_zero() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let body = br#"{"outputs":[{"shape":[2,1],"data":[1.5,2.5]},{"shape":[1,1],"data":[3.5]}]}"#;
    let reply = transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap();
    let indices: Vec<u32> = reply.data.iter().map(|record| record.index).collect();
    assert_eq!(indices, [0, 1, 0]);
}

#[test]
fn embedding_response_rejects_a_body_it_cannot_read() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let error = transform_embedding_response(502, b"not json", "model", &request, &WordsCounter)
        .unwrap_err();
    assert!(matches!(
        error,
        TritonError::Api {
            status_code: 502,
            ..
        }
    ));
    assert_eq!(error.status(), Some(502));
    let wrong_shape =
        transform_embedding_response(200, br#"{"outputs":{}}"#, "model", &request, &WordsCounter)
            .unwrap_err();
    assert!(matches!(wrong_shape, TritonError::ReplyShape { .. }));
}

#[rstest]
#[case::array_body(br#"[[{"shape":[1,1],"data":[1.5]}]]"#.as_slice())]
#[case::array_output(br#"{"outputs":[[[1,1],[1.5]]]}"#.as_slice())]
fn embedding_response_rejects_an_array_where_an_object_belongs(#[case] body: &[u8]) {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let error =
        transform_embedding_response(200, body, "model", &request, &WordsCounter).unwrap_err();
    assert!(matches!(error, TritonError::ReplyShape { .. }));
    assert_eq!(error.status(), None);
}

#[rstest]
#[case::object(
    r#"{"text_output":"hi","stop_reason":"stop","is_finished":true,"index":2}"#,
    "hi",
    true,
    "stop",
    2
)]
#[case::missing_fields(r#"{"text_output":"hi"}"#, "hi", false, "", 0)]
#[case::empty_object("{}", "", false, "", 0)]
#[case::data_prefix(
    r#"data: {"text_output":"hi","is_finished":false}"#,
    "hi",
    false,
    "",
    0
)]
#[case::data_without_space(r#"data:{"text_output":"hi"}"#, "hi", false, "", 0)]
#[case::not_json("data: not json", "", false, "", 0)]
#[case::empty_line("", "", false, "", 0)]
#[case::null("data: null", "", false, "", 0)]
#[case::zero("data: 0", "", false, "", 0)]
#[case::zero_float("data: 0.0", "", false, "", 0)]
#[case::negative_zero_float("data: -0.0", "", false, "", 0)]
#[case::false_value("data: false", "", false, "", 0)]
#[case::empty_array("data: []", "", false, "", 0)]
#[case::empty_string(r#"data: """#, "", false, "", 0)]
fn stream_lines_decode_like_the_chunk_parser(
    #[case] line: &str,
    #[case] text: &str,
    #[case] is_finished: bool,
    #[case] finish_reason: &str,
    #[case] index: i64,
) {
    let chunk = decode_stream_line(line).unwrap();
    assert_eq!(chunk.text, text);
    assert_eq!(chunk.is_finished, is_finished);
    assert_eq!(chunk.finish_reason, finish_reason);
    assert_eq!(chunk.index, index);
}

#[rstest]
#[case::done("[DONE]")]
#[case::sse_done("data: [DONE]")]
#[case::done_inside_the_object(r#"data: {"text_output":"a[DONE]b"}"#)]
fn stream_done_lines_finish_the_stream(#[case] line: &str) {
    let chunk = decode_stream_line(line).unwrap();
    assert_eq!(chunk.text, "");
    assert!(chunk.is_finished);
    assert_eq!(chunk.finish_reason, "stop");
    assert_eq!(chunk.index, 0);
}

#[rstest]
#[case::mixed_type(r#"data: {"text_output":5}"#)]
#[case::null_text(r#"data: {"text_output":null}"#)]
#[case::bad_index(r#"data: {"index":1.5}"#)]
#[case::array("data: [1,2]")]
#[case::array_of_the_fields(r#"data: ["hi"]"#)]
#[case::array_of_every_field(r#"data: ["hi","stop",true,1]"#)]
#[case::truthy_string(r#"data: "hi""#)]
#[case::true_value("data: true")]
#[case::scalar("data: 5")]
#[case::float("data: 1.5")]
fn stream_lines_reject_a_field_of_the_wrong_type(#[case] line: &str) {
    let error = decode_stream_line(line).unwrap_err();
    assert!(matches!(error, TritonError::ReplyShape { .. }));
    assert_eq!(error.status(), None);
}

#[tokio::test]
async fn router_sends_generate_and_infer_by_the_api_base() {
    let prompts = factory();
    let given = user_message();
    let optional = options(r#"{"max_tokens":5}"#);
    let generate = route_request(
        &config(&prompts),
        "https://host/generate",
        &input("meta-llama/llama-2-7b-chat", &given, &optional),
    )
    .await
    .unwrap();
    assert!(matches!(generate, TritonRequest::Generate(_)));
    let infer = route_request(
        &config(&prompts),
        "https://host/infer",
        &input("model", &given, &optional),
    )
    .await
    .unwrap();
    assert!(matches!(infer, TritonRequest::Infer(_)));
    route_request(
        &config(&prompts),
        "https://host/other",
        &input("model", &given, &optional),
    )
    .await
    .unwrap_err();
}

#[test]
fn router_decodes_generate_and_infer_replies_by_the_api_base() {
    let generate =
        route_response("https://host/generate", 200, br#"{"text_output":"hi"}"#).unwrap();
    assert!(matches!(generate, TritonReply::Generate(_)));
    let infer =
        route_response("https://host/infer", 200, br#"{"outputs":[{"data":"hi"}]}"#).unwrap();
    assert!(matches!(infer, TritonReply::Infer(_)));
    route_response("https://host/other", 200, b"{}").unwrap_err();
}

#[test]
fn replays_the_triton_reply_corpus() {
    let request = transform_embedding_request(EmbeddingInput::Texts(vec![String::from("hello")]));
    let seen = fs::read_dir("fuzz/corpus/triton_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let _generated = generate_response(200, &bytes);
            let _inferred = infer_response(200, &bytes);
            let _embedding =
                transform_embedding_response(200, &bytes, "model", &request, &WordsCounter);
            if let Ok(line) = str::from_utf8(&bytes) {
                let _chunk = decode_stream_line(line);
            }
        })
        .count();
    assert!(seen > 0_usize);
}
