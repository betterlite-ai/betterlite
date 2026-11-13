#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, from_str, json};

use super::PrepareRequest;
use super::error::HuggingfaceEmbeddingError;
use super::prepare;
use super::request::{Params, input_for_tag, process_optional_params, transform_input};
use super::response::{
    EmbeddingReply, MAX_RESPONSE_BYTES, OrdinaryEncoder, decode_embedding_response,
    process_embedding_response,
};
use super::task::{PipelineTagSource, resolve_task};
use super::url::embedding_url;
use crate::huggingface_chat::HuggingfaceEnv;
use crate::huggingface_chat::error::HuggingfaceError;
use crate::openai_base_config::{Headers, Object};

struct RecordedSource {
    tag: Option<String>,
    failure: Option<HuggingfaceError>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl RecordedSource {
    fn returning(tag: Option<&str>) -> Self {
        Self {
            tag: tag.map(str::to_owned),
            failure: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            tag: None,
            failure: Some(HuggingfaceError {
                status_code,
                message: "the Hub is down".to_owned(),
                headers: Headers::new(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PipelineTagSource for RecordedSource {
    fn tag(
        &self,
        model: &str,
        task_type: Option<&str>,
    ) -> Result<Option<String>, HuggingfaceError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), task_type.map(str::to_owned)));
        self.failure
            .as_ref()
            .map_or_else(|| Ok(self.tag.clone()), |failure| Err(failure.clone()))
    }
}

struct FixedCount(usize);

impl OrdinaryEncoder for FixedCount {
    fn encode_ordinary(&self, _text: &str) -> usize {
        self.0
    }
}

fn env(hf_api_base: Option<&str>, huggingface_api_base: Option<&str>) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: hf_api_base.map(str::to_owned),
        huggingface_api_base: huggingface_api_base.map(str::to_owned),
    }
}

fn texts(source: &str) -> Vec<String> {
    from_str(source).unwrap()
}

fn options(source: &str) -> Params {
    from_str(source).unwrap()
}

fn value(source: &str) -> Value {
    from_str(source).unwrap()
}

fn object(source: &str) -> Object {
    from_str(source).unwrap()
}

fn status_of(error: &HuggingfaceEmbeddingError) -> Option<u16> {
    match error {
        HuggingfaceEmbeddingError::Huggingface(source) => Some(source.status_code),
        HuggingfaceEmbeddingError::InvalidTaskType { .. }
        | HuggingfaceEmbeddingError::ResponseTooLarge { .. }
        | HuggingfaceEmbeddingError::ResponseDecode { .. }
        | HuggingfaceEmbeddingError::RequestEncode { .. }
        | HuggingfaceEmbeddingError::SectionNotObject { .. }
        | HuggingfaceEmbeddingError::EmbeddingShape { .. }
        | HuggingfaceEmbeddingError::UsageOverflow => None,
    }
}

fn pair(first: &str, sentences: &[&str]) -> Value {
    Value::Object(Object::from_iter([
        (
            "source_sentence".to_owned(),
            Value::String(first.to_owned()),
        ),
        (
            "sentences".to_owned(),
            Value::Array(
                sentences
                    .iter()
                    .map(|text| Value::String((*text).to_owned()))
                    .collect(),
            ),
        ),
    ]))
}

fn at(body: &Object, path: &str) -> Value {
    Value::Object(body.clone())
        .pointer(path)
        .cloned()
        .unwrap_or(Value::Null)
}

fn prepared_request(
    source: &RecordedSource,
    model: &str,
    input: &[String],
    optional_params: &Params,
    environment: &HuggingfaceEnv,
) -> Result<super::PreparedRequest, HuggingfaceEmbeddingError> {
    let headers = Headers::new();
    let request = PrepareRequest {
        model,
        input,
        optional_params,
        headers: &headers,
        api_key: None,
        api_base: None,
        env: environment,
    };
    prepare(&request, source)
}

#[rstest]
#[case::sentence_similarity("sentence-similarity")]
#[case::feature_extraction("feature-extraction")]
#[case::rerank("rerank")]
#[case::embed("embed")]
#[case::similarity("similarity")]
fn task_accepts_every_embedding_value(#[case] task: &str) {
    let source = RecordedSource::returning(Some("text-generation"));
    let value = json!(task);
    let resolved = resolve_task("org/model", Some(&value), &source).unwrap();
    assert_eq!(resolved, Some(task.to_owned()));
    assert!(source.calls().is_empty());
}

#[rstest]
#[case::other_tag(json!("text-generation"))]
#[case::number(json!(5_u64))]
#[case::boolean(json!(true))]
#[case::list(json!(["embed"]))]
fn task_rejects_any_other_value(#[case] value: Value) {
    let source = RecordedSource::returning(Some("embed"));
    let error = resolve_task("org/model", Some(&value), &source).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::InvalidTaskType { .. }
    ));
    assert!(source.calls().is_empty());
}

#[test]
fn task_reads_the_pipeline_tag_without_a_type() {
    let source = RecordedSource::returning(Some("rerank"));
    assert_eq!(
        resolve_task("org/model", None, &source).unwrap(),
        Some("rerank".to_owned())
    );
    assert_eq!(source.calls(), vec![("org/model".to_owned(), None)]);
}

#[test]
fn task_treats_a_null_type_as_absent() {
    let source = RecordedSource::returning(Some("embed"));
    let null = Value::Null;
    assert_eq!(
        resolve_task("org/model", Some(&null), &source).unwrap(),
        Some("embed".to_owned())
    );
    assert_eq!(source.calls().len(), 1);
}

#[test]
fn task_propagates_a_source_failure() {
    let source = RecordedSource::failing(503);
    let error = resolve_task("org/model", None, &source).unwrap_err();
    assert_eq!(status_of(&error), Some(503));
}

#[rstest]
#[case::plain_model("BAAI/bge-small-en-v1.5", 2)]
#[case::sentence_transformers("sentence-transformers/all-MiniLM-L6-v2", 1)]
fn task_prepare_looks_up_twice_unless_sentence_transformers(
    #[case] model: &str,
    #[case] expected: usize,
) {
    let source = RecordedSource::returning(Some("feature-extraction"));
    let input = texts(r#"["first", "second"]"#);
    let params = Params::new();
    let environment = HuggingfaceEnv::default();
    let prepared = prepared_request(&source, model, &input, &params, &environment).unwrap();
    assert_eq!(source.calls().len(), expected);
    assert!(prepared.url.contains("/feature-extraction/"));
}

#[test]
fn task_prepare_prefers_the_input_type_over_the_hub_tag() {
    let source = RecordedSource::returning(Some("feature-extraction"));
    let input = texts(r#"["first", "second"]"#);
    let params = options(r#"{"input_type": "rerank"}"#);
    let environment = HuggingfaceEnv::default();
    let prepared = prepared_request(&source, "org/model", &input, &params, &environment).unwrap();
    assert!(source.calls().is_empty());
    assert!(prepared.url.ends_with("/rerank/org/model"));
    let body: Value = from_str(&prepared.body).unwrap();
    assert_eq!(
        body.get("inputs"),
        Some(&value(r#"{"query": "first", "texts": ["second"]}"#))
    );
}

#[rstest]
#[case::http("http://model.example/embed")]
#[case::https("https://model.example/embed")]
fn url_a_model_url_wins(#[case] model: &str) {
    let environment = env(Some("https://hf.example"), Some("https://other.example"));
    assert_eq!(
        embedding_url(
            model,
            Some("https://api.example"),
            &environment,
            Some("embed")
        ),
        model
    );
}

#[rstest]
#[case::api_base(Some("https://api.example"), "https://api.example")]
#[case::empty_api_base(Some(""), "https://hf.example")]
#[case::no_api_base(None, "https://hf.example")]
fn url_api_base_or_the_environment(#[case] api_base: Option<&str>, #[case] expected: &str) {
    let environment = env(Some("https://hf.example"), None);
    assert_eq!(
        embedding_url("org/model", api_base, &environment, None),
        expected
    );
}

#[rstest]
#[case::hf_first(
    Some("https://hf.example"),
    Some("https://other.example"),
    "https://hf.example"
)]
#[case::huggingface_only(None, Some("https://other.example"), "https://other.example")]
#[case::empty_hf_is_present(Some(""), Some("https://other.example"), "")]
#[case::empty_huggingface_is_present(None, Some(""), "")]
fn url_environment_bases_follow_presence_order(
    #[case] hf_api_base: Option<&str>,
    #[case] huggingface_api_base: Option<&str>,
    #[case] expected: &str,
) {
    let environment = env(hf_api_base, huggingface_api_base);
    assert_eq!(
        embedding_url("org/model", None, &environment, None),
        expected
    );
}

#[rstest]
#[case::task(
    Some("feature-extraction"),
    "https://router.huggingface.co/hf-inference/pipeline/feature-extraction/org/model"
)]
#[case::absent_task(
    None,
    "https://router.huggingface.co/hf-inference/pipeline/None/org/model"
)]
fn url_the_router_default_carries_the_task(#[case] task: Option<&str>, #[case] expected: &str) {
    let environment = HuggingfaceEnv::default();
    assert_eq!(
        embedding_url("org/model", None, &environment, task),
        expected
    );
}

#[test]
fn url_headers_merge_the_defaults_over_the_caller() {
    let source = RecordedSource::returning(Some("embed"));
    let input = texts(r#"["text"]"#);
    let params = Params::new();
    let headers: Headers = from_str(r#"{"content-type": "text/plain", "x-call": "kept"}"#).unwrap();
    let key = SecretString::from("sk-hf");
    let environment = HuggingfaceEnv::default();
    let request = PrepareRequest {
        model: "org/model",
        input: &input,
        optional_params: &params,
        headers: &headers,
        api_key: Some(&key),
        api_base: None,
        env: &environment,
    };
    let prepared = prepare(&request, &source).unwrap();
    assert_eq!(
        prepared.headers.get("content-type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(
        prepared.headers.get("Authorization"),
        Some(&"Bearer sk-hf".to_owned())
    );
    assert_eq!(prepared.headers.get("x-call"), Some(&"kept".to_owned()));
}

#[test]
fn url_headers_add_a_bearer_for_an_empty_key_and_none_without_one() {
    let source = RecordedSource::returning(Some("embed"));
    let input = texts(r#"["text"]"#);
    let params = Params::new();
    let headers = Headers::new();
    let empty = SecretString::from("");
    let environment = HuggingfaceEnv::default();
    let with_key = PrepareRequest {
        model: "org/model",
        input: &input,
        optional_params: &params,
        headers: &headers,
        api_key: Some(&empty),
        api_base: None,
        env: &environment,
    };
    let with_empty = prepare(&with_key, &source).unwrap();
    assert_eq!(
        with_empty.headers.get("Authorization"),
        Some(&"Bearer ".to_owned())
    );
    let without_key = PrepareRequest {
        model: "org/model",
        input: &input,
        optional_params: &params,
        headers: &headers,
        api_key: None,
        api_base: None,
        env: &environment,
    };
    let without = prepare(&without_key, &source).unwrap();
    assert!(!without.headers.contains_key("Authorization"));
    assert_eq!(
        without.headers.get("content-type"),
        Some(&"application/json".to_owned())
    );
}

#[rstest]
#[case::absent(None)]
#[case::feature_extraction(Some("feature-extraction"))]
#[case::embed(Some("embed"))]
#[case::other(Some("text-generation"))]
fn body_plain_tags_send_the_input_list(#[case] tag: Option<&str>) {
    let input = texts(r#"["one", "two"]"#);
    let body = input_for_tag(&input, tag).unwrap();
    assert_eq!(body.get("inputs"), Some(&json!(["one", "two"])));
}

#[test]
fn body_sentence_similarity_builds_the_source_pair() {
    let input = texts(r#"["first", "second", "third"]"#);
    let body = input_for_tag(&input, Some("sentence-similarity")).unwrap();
    assert_eq!(at(&body, "/inputs/source_sentence"), json!("first"));
    assert_eq!(at(&body, "/inputs/sentences"), json!(["second", "third"]));
}

#[test]
fn body_similarity_builds_the_same_pair() {
    let input = texts(r#"["first", "second"]"#);
    let body = input_for_tag(&input, Some("similarity")).unwrap();
    assert_eq!(at(&body, "/inputs/source_sentence"), json!("first"));
    assert_eq!(at(&body, "/inputs/sentences"), json!(["second"]));
}

#[rstest]
#[case::empty("[]")]
#[case::single(r#"["only"]"#)]
fn body_sentence_similarity_requires_two_inputs(#[case] source: &str) {
    let input = texts(source);
    let error = input_for_tag(&input, Some("sentence-similarity")).unwrap_err();
    assert_eq!(status_of(&error), Some(400));
    assert!(
        error
            .to_string()
            .contains("sentence-similarity requires 2+ sentences")
    );
}

#[rstest]
#[case::empty("[]")]
#[case::single(r#"["only"]"#)]
fn body_rerank_requires_two_inputs(#[case] source: &str) {
    let input = texts(source);
    let error = input_for_tag(&input, Some("rerank")).unwrap_err();
    assert_eq!(status_of(&error), Some(400));
    assert!(error.to_string().contains("reranker requires 2+ sentences"));
}

#[test]
fn body_rerank_builds_query_and_texts() {
    let input = texts(r#"["question", "first document", "second document"]"#);
    let body = input_for_tag(&input, Some("rerank")).unwrap();
    assert_eq!(at(&body, "/inputs/query"), json!("question"));
    assert_eq!(
        at(&body, "/inputs/texts"),
        json!(["first document", "second document"])
    );
}

#[rstest]
#[case::two_inputs(r#"["first", "second"]"#, pair("first", &["second"]))]
#[case::single_input(r#"["only"]"#, pair("only", &[]))]
fn body_sentence_transformers_builds_the_pair(#[case] source: &str, #[case] expected: Value) {
    let input = texts(source);
    let params = Params::new();
    let recorder = RecordedSource::returning(Some("rerank"));
    let body = transform_input(
        "sentence-transformers/all-MiniLM-L6-v2",
        &input,
        &params,
        &recorder,
    )
    .unwrap();
    assert_eq!(body.get("inputs"), Some(&expected));
    assert!(recorder.calls().is_empty());
}

#[test]
fn body_sentence_transformers_keeps_input_type_at_the_top_level() {
    let input = texts(r#"["first", "second"]"#);
    let params = options(r#"{"input_type": "query", "wait_for_model": true}"#);
    let recorder = RecordedSource::returning(Some("rerank"));
    let body = transform_input(
        "org/sentence-transformers-model",
        &input,
        &params,
        &recorder,
    )
    .unwrap();
    assert_eq!(body.get("input_type"), Some(&json!("query")));
    assert_eq!(at(&body, "/options/wait_for_model"), json!(true));
    assert!(recorder.calls().is_empty());
}

#[test]
fn body_sentence_transformers_rejects_an_empty_input() {
    let input: Vec<String> = Vec::new();
    let params = Params::new();
    let recorder = RecordedSource::returning(None);
    let error = transform_input(
        "sentence-transformers/all-MiniLM-L6-v2",
        &input,
        &params,
        &recorder,
    )
    .unwrap_err();
    assert_eq!(status_of(&error), Some(400));
    assert!(
        error
            .to_string()
            .contains("sentence transformers requires 2+ sentences")
    );
}

#[test]
fn body_options_and_parameters_are_placed_by_name() {
    let input = texts(r#"["text"]"#);
    let params = options(
        r#"{"use_cache": true, "wait_for_model": false, "min_length": 1, "max_length": 8, "top_k": 5, "top_p": 0.9, "temperature": 0.7, "repetition_penalty": 1.1, "max_time": 30}"#,
    );
    let recorder = RecordedSource::returning(Some("feature-extraction"));
    let body = transform_input("org/model", &input, &params, &recorder).unwrap();
    assert_eq!(at(&body, "/options/use_cache"), json!(true));
    assert_eq!(at(&body, "/options/wait_for_model"), json!(false));
    assert_eq!(at(&body, "/parameters/min_length"), json!(1_u64));
    assert_eq!(at(&body, "/parameters/max_length"), json!(8_u64));
    assert_eq!(at(&body, "/parameters/top_k"), json!(5_u64));
    assert_eq!(at(&body, "/parameters/top_p"), json!(0.9_f64));
    assert_eq!(at(&body, "/parameters/temperature"), json!(0.7_f64));
    assert_eq!(at(&body, "/parameters/repetition_penalty"), json!(1.1_f64));
    assert_eq!(at(&body, "/parameters/max_time"), json!(30_u64));
    assert!(!body.contains_key("use_cache"));
    assert!(!body.contains_key("min_length"));
}

#[test]
fn body_other_params_stay_at_the_top_level() {
    let input = texts(r#"["text"]"#);
    let params = options(r#"{"dimensions": 8, "input_type": "embed"}"#);
    let recorder = RecordedSource::returning(None);
    let body = transform_input("org/model", &input, &params, &recorder).unwrap();
    assert_eq!(body.get("inputs"), Some(&json!(["text"])));
    assert_eq!(body.get("dimensions"), Some(&json!(8_u64)));
    assert!(!body.contains_key("input_type"));
}

#[test]
fn body_process_optional_params_merges_into_an_existing_section() {
    let data = object(r#"{"inputs": ["text"], "options": {"keep": 1}}"#);
    let params = options(r#"{"use_cache": true}"#);
    let merged = process_optional_params(data, &params).unwrap();
    assert_eq!(at(&merged, "/options/keep"), json!(1_u64));
    assert_eq!(at(&merged, "/options/use_cache"), json!(true));
}

#[rstest]
#[case::options_last(
    r#"{"use_cache": true, "options": {"keep": 1}}"#,
    "/options",
    value(r#"{"keep": 1}"#)
)]
#[case::parameters_last(
    r#"{"max_length": 8, "parameters": {"truncate": true}}"#,
    "/parameters",
    value(r#"{"truncate": true}"#)
)]
#[case::options_first(
    r#"{"options": {"keep": 1}, "use_cache": true}"#,
    "/options",
    value(r#"{"keep": 1, "use_cache": true}"#)
)]
#[case::parameters_first(
    r#"{"parameters": {"truncate": true}, "max_length": 8}"#,
    "/parameters",
    value(r#"{"truncate": true, "max_length": 8}"#)
)]
#[case::string_section_last(r#"{"use_cache": true, "options": "text"}"#, "/options", json!("text"))]
fn body_optional_params_apply_in_the_order_sent(
    #[case] params: &str,
    #[case] pointer: &str,
    #[case] expected: Value,
) {
    let data = object(r#"{"inputs": ["text"]}"#);
    let merged = process_optional_params(data, &options(params)).unwrap();
    assert_eq!(at(&merged, pointer), expected);
}

#[test]
fn body_transform_input_keeps_the_order_after_removing_input_type() {
    let input = texts(r#"["text"]"#);
    let params = options(r#"{"max_length": 8, "input_type": "embed", "parameters": {"a": 1}}"#);
    let recorder = RecordedSource::returning(None);
    let body = transform_input("org/model", &input, &params, &recorder).unwrap();
    assert_eq!(at(&body, "/parameters"), value(r#"{"a": 1}"#));
}

#[rstest]
#[case::options(r#"{"options": "text", "use_cache": true}"#, "options")]
#[case::parameters(r#"{"parameters": [], "top_k": 3}"#, "parameters")]
fn body_a_section_collision_is_an_error(#[case] params: &str, #[case] section: &str) {
    let data = object(r#"{"inputs": ["text"]}"#);
    let error = process_optional_params(data, &options(params)).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::SectionNotObject { section: found } if found == section
    ));
}

#[test]
fn body_prepare_serializes_the_body_as_json() {
    let source = RecordedSource::returning(Some("sentence-similarity"));
    let input = texts(r#"["first", "second"]"#);
    let params = Params::new();
    let environment = HuggingfaceEnv::default();
    let prepared = prepared_request(&source, "org/model", &input, &params, &environment).unwrap();
    let body: Value = from_str(&prepared.body).unwrap();
    assert_eq!(
        body.pointer("/inputs/source_sentence"),
        Some(&json!("first"))
    );
    assert_eq!(body.pointer("/inputs/sentences"), Some(&json!(["second"])));
}

#[rstest]
#[case::vectors("[[0.1, 0.2], [0.3, 0.4]]", &[json!([0.1_f64, 0.2_f64]), json!([0.3_f64, 0.4_f64])])]
#[case::scalars("[0.5, 0.25]", &[json!(0.5_f64), json!(0.25_f64)])]
#[case::nested("[[[0.1, 0.2]], [[0.3, 0.4]]]", &[json!(0.1_f64), json!(0.3_f64)])]
#[case::integers("[1, [2, 3]]", &[json!(1_u64), json!([2_u64, 3_u64])])]
#[case::single_vector("[[0.1, 0.2]]", &[json!([0.1_f64, 0.2_f64])])]
#[case::single_scalar("[0.5]", &[json!(0.5_f64)])]
fn response_each_element_becomes_a_record(#[case] source: &str, #[case] expected: &[Value]) {
    let reply: EmbeddingReply = from_str(source).unwrap();
    let input = texts(r#"["hello"]"#);
    let response = process_embedding_response(&reply, "org/model", &input, &FixedCount(2)).unwrap();
    let embeddings: Vec<Value> = response
        .data
        .iter()
        .map(|record| record.embedding.clone())
        .collect();
    assert_eq!(embeddings, expected);
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "org/model");
    assert_eq!(response.data[0].object, "embedding");
    assert_eq!(response.data[0].index, 0);
}

#[test]
fn response_a_mixed_list_keeps_each_shape() {
    let reply: EmbeddingReply = from_str("[0.5, [0.1, 0.2], [[0.3, 0.4]]]").unwrap();
    let response = process_embedding_response(&reply, "m", &[], &FixedCount(0)).unwrap();
    let embeddings: Vec<Value> = response
        .data
        .iter()
        .map(|record| record.embedding.clone())
        .collect();
    assert_eq!(
        embeddings,
        vec![json!(0.5_f64), json!([0.1_f64, 0.2_f64]), json!(0.3_f64)]
    );
}

#[rstest]
#[case::empty_element("[[]]")]
#[case::empty_nested("[[[]]]")]
#[case::string_element(r#"["text"]"#)]
fn response_an_element_without_a_value_is_an_error(#[case] source: &str) {
    let reply: EmbeddingReply = from_str(source).unwrap();
    let error = process_embedding_response(&reply, "m", &[], &FixedCount(0)).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::EmbeddingShape { index: 0 }
    ));
    assert_eq!(status_of(&error), None);
}

#[test]
fn response_similarities_become_indexed_records() {
    let reply: EmbeddingReply =
        from_str(r#"{"similarities": [[1, [0.1, 0.2]], [2, 0.5]]}"#).unwrap();
    let response = process_embedding_response(&reply, "m", &[], &FixedCount(0)).unwrap();
    assert_eq!(response.data[0].index, 1);
    assert_eq!(response.data[0].embedding, json!([0.1_f64, 0.2_f64]));
    assert_eq!(response.data[1].index, 2);
    assert_eq!(response.data[1].embedding, json!(0.5_f64));
}

#[test]
fn response_usage_sums_every_input_and_sets_every_count() {
    let reply: EmbeddingReply = from_str("[[0.1]]").unwrap();
    let input = texts(r#"["hello", "second text", "third"]"#);
    let response = process_embedding_response(&reply, "m", &input, &FixedCount(2)).unwrap();
    assert_eq!(response.usage.prompt_tokens, 6);
    assert_eq!(response.usage.completion_tokens, 6);
    assert_eq!(response.usage.total_tokens, 6);
}

#[test]
fn response_an_error_key_is_a_500() {
    let reply: EmbeddingReply = from_str(r#"{"error": "model is loading"}"#).unwrap();
    let error = process_embedding_response(&reply, "m", &[], &FixedCount(0)).unwrap_err();
    assert_eq!(status_of(&error), Some(500));
    assert!(error.to_string().contains("model is loading"));
}

#[test]
fn response_decoding_rejects_an_oversized_body() {
    let bytes = vec![b' '; 0x400_0001];
    let error = decode_embedding_response(&bytes).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::ResponseTooLarge { limit } if limit == MAX_RESPONSE_BYTES
    ));
}

#[test]
fn response_decoding_accepts_a_body_of_exactly_the_limit() {
    let mut bytes = vec![b' '; 0x400_0000];
    bytes[0] = b'[';
    bytes[0x3FF_FFFF] = b']';
    assert!(matches!(
        decode_embedding_response(&bytes).unwrap(),
        EmbeddingReply::Items(items) if items.is_empty()
    ));
}

#[test]
fn response_decoding_rejects_an_object_without_a_known_key() {
    let error = decode_embedding_response(br#"{"object": "list"}"#).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::ResponseDecode { .. }
    ));
}

#[test]
fn response_decoding_rejects_malformed_json() {
    let error = decode_embedding_response(br#"{"similarities": [}"#).unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceEmbeddingError::ResponseDecode { .. }
    ));
    assert_eq!(status_of(&error), None);
}

#[test]
fn response_decoding_accepts_the_three_shapes() {
    assert!(matches!(
        decode_embedding_response(br#"{"error": "x"}"#).unwrap(),
        EmbeddingReply::Failure(_)
    ));
    assert!(matches!(
        decode_embedding_response(br#"{"similarities": [[0, [0.1]]]}"#).unwrap(),
        EmbeddingReply::Similarities(_)
    ));
    assert!(matches!(
        decode_embedding_response(b"[[0.1]]").unwrap(),
        EmbeddingReply::Items(_)
    ));
}

#[rstest]
#[case::vector(b"[[0.1, 0.2]]".as_slice(), json!([0.1_f64, 0.2_f64]))]
#[case::scalar(b"[0.5]".as_slice(), json!(0.5_f64))]
#[case::nested(b"[[[0.1, 0.2]]]".as_slice(), json!(0.1_f64))]
fn response_decoding_reads_a_single_element_list(#[case] bytes: &[u8], #[case] expected: Value) {
    let reply = decode_embedding_response(bytes).unwrap();
    assert!(matches!(reply, EmbeddingReply::Items(_)));
    let input = texts(r#"["hello"]"#);
    let response = process_embedding_response(&reply, "m", &input, &FixedCount(1)).unwrap();
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].embedding, expected);
}

#[test]
fn response_decoding_prefers_the_error_key() {
    let reply = decode_embedding_response(br#"{"error": "x", "similarities": []}"#).unwrap();
    assert!(matches!(reply, EmbeddingReply::Failure(_)));
}

#[rstest]
fn replays_the_embedding_response_corpus(
    #[files("fuzz/corpus/huggingface_embedding_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _decoded = decode_embedding_response(&bytes);
}
