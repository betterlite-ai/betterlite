#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;
use secrecy::SecretString;

use super::error::{HuggingfaceRerankError, MAX_RESPONSE_BYTES};
use super::headers::validate_environment;
use super::params::{
    Params, SUPPORTED_RERANK_PARAMS, map_cohere_rerank_params, transform_rerank_request,
};
use super::response::{
    RerankResponseItem, ResponseRequest, decode_rerank_response, transform_rerank_response,
};
use super::url::{get_api_base, get_complete_url};
use super::{RequestIdSource, TextTokenCounter};
use crate::api::rerank_contracts::records::{
    RerankResponse, RerankResponseDocument, RerankResponseMeta, RerankResponseResult, Score,
};
use crate::huggingface_chat::HuggingfaceEnv;
use crate::openai_base_config::Headers;

const MODEL: &str = "model";

struct RecordedCounter {
    remaining: Mutex<VecDeque<Option<u64>>>,
    seen: Mutex<Vec<(String, String)>>,
}

impl RecordedCounter {
    fn new(counts: &[Option<u64>]) -> Self {
        Self {
            remaining: Mutex::new(counts.iter().copied().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl TextTokenCounter for RecordedCounter {
    fn count(&self, model: &str, text: &str) -> Result<u64, CountError> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), text.to_owned()));
        match self
            .remaining
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
        {
            Some(Some(value)) => Ok(value),
            Some(None) | None => Err(CountError::MissingInput),
        }
    }
}

struct FixedId(&'static str);

impl RequestIdSource for FixedId {
    fn new_id(&self) -> String {
        self.0.to_owned()
    }
}

fn env(hf_api_base: Option<&str>, huggingface_api_base: Option<&str>) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: hf_api_base.map(str::to_owned),
        huggingface_api_base: huggingface_api_base.map(str::to_owned),
    }
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).unwrap()
}

fn score(value: f64) -> Score {
    Score::try_from(value).unwrap()
}

fn document(text: &str) -> RerankResponseDocument {
    RerankResponseDocument {
        text: text.to_owned(),
    }
}

fn headers(source: &str) -> Headers {
    serde_json::from_str(source).unwrap()
}

fn run_response(
    body: &str,
    status: Option<u16>,
    request_data: &str,
    counts: &[Option<u64>],
    id: &str,
) -> Result<RerankResponse, HuggingfaceRerankError> {
    let request_data = params(request_data);
    let counter = RecordedCounter::new(counts);
    let request = ResponseRequest {
        model: MODEL,
        status_code: status,
        body: body.as_bytes(),
        request_data: &request_data,
    };
    transform_rerank_response(&request, &counter, &FixedId(id))
}

fn success(body: &str, request_data: &str, counts: &[Option<u64>]) -> RerankResponse {
    run_response(body, Some(200), request_data, counts, "fixed-id").unwrap()
}

fn results(response: &RerankResponse) -> Vec<RerankResponseResult> {
    response.results.clone().unwrap_or_default()
}

fn meta(response: &RerankResponse) -> RerankResponseMeta {
    response.meta.clone().unwrap_or_default()
}

fn tokens(response: &RerankResponse) -> (i64, i64) {
    let tokens = meta(response)
        .tokens
        .unwrap_or_default()
        .unwrap_or_default();
    (
        tokens.input_tokens.unwrap_or_default().unwrap_or_default(),
        tokens.output_tokens.unwrap_or_default().unwrap_or_default(),
    )
}

#[rstest]
#[case::given(Some("https://given.example"), None, None, "https://given.example")]
#[case::hf(None, Some("https://hf.example"), None, "https://hf.example")]
#[case::huggingface(None, Some("https://other.example"), None, "https://other.example")]
#[case::default(None, None, None, "https://api-inference.huggingface.co")]
#[case::empty_hf(None, Some(""), None, "")]
fn url_selects_the_api_base(
    #[case] api_base: Option<&str>,
    #[case] hf_api_base: Option<&str>,
    #[case] huggingface_api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        get_api_base(api_base, &env(hf_api_base, huggingface_api_base)),
        expected
    );
}

#[rstest]
#[case::plain(None, "https://api-inference.huggingface.co/rerank")]
#[case::trailing_slash(Some("https://host.example/"), "https://host.example/rerank")]
#[case::many_slashes(Some("https://host.example///"), "https://host.example/rerank")]
#[case::already_rerank(Some("https://host.example/rerank"), "https://host.example/rerank")]
#[case::rerank_with_slash(Some("https://host.example/rerank/"), "https://host.example/rerank")]
#[case::empty_base(Some(""), "/rerank")]
fn url_completes_the_rerank_path(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(get_complete_url(api_base, &env(None, None)), expected);
}

#[test]
fn url_keeps_an_environment_base_that_already_ends_in_rerank() {
    assert_eq!(
        get_complete_url(None, &env(Some("https://hf.example/rerank"), None)),
        "https://hf.example/rerank"
    );
}

#[test]
fn request_supported_params_are_the_cohere_four() {
    assert_eq!(
        SUPPORTED_RERANK_PARAMS,
        ["query", "documents", "top_n", "return_documents"]
    );
}

#[test]
fn request_maps_documents_to_texts() {
    assert_eq!(
        map_cohere_rerank_params(Some(&params(r#"{"documents": ["a", {"text": "b"}]}"#))),
        params(r#"{"texts": ["a", {"text": "b"}]}"#)
    );
}

#[rstest]
#[case::yes(r#"{"return_documents": true}"#, r#"{"return_text": true}"#)]
#[case::no(r#"{"return_documents": false}"#, r#"{"return_text": false}"#)]
fn request_maps_a_bool_return_documents_to_return_text(
    #[case] source: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        map_cohere_rerank_params(Some(&params(source))),
        params(expected)
    );
}

#[rstest]
#[case::text(r#"{"return_documents": "yes"}"#)]
#[case::number(r#"{"return_documents": 1}"#)]
#[case::list(r#"{"return_documents": []}"#)]
fn request_drops_a_return_documents_that_is_not_a_bool(#[case] source: &str) {
    assert_eq!(
        map_cohere_rerank_params(Some(&params(source))),
        Params::new()
    );
}

#[test]
fn request_copies_top_n_and_query() {
    assert_eq!(
        map_cohere_rerank_params(Some(&params(r#"{"top_n": 2, "query": "hi"}"#))),
        params(r#"{"top_n": 2, "query": "hi"}"#)
    );
}

#[test]
fn request_drops_null_and_unknown_values() {
    assert_eq!(
        map_cohere_rerank_params(Some(&params(
            r#"{"query": null, "documents": null, "top_n": null, "rank_fields": ["a"], "instruction": "x"}"#
        ))),
        Params::new()
    );
}

#[test]
fn request_maps_a_missing_param_map_to_nothing() {
    assert_eq!(map_cohere_rerank_params(None), Params::new());
}

#[test]
fn request_body_keeps_the_three_defaults() {
    assert_eq!(
        transform_rerank_request(&params(r#"{"query": "q", "texts": ["d"]}"#)).unwrap(),
        params(
            r#"{"query": "q", "texts": ["d"], "raw_scores": false, "truncate": false, "truncation_direction": "Right"}"#
        )
    );
}

#[test]
fn request_body_lets_params_override_the_defaults() {
    assert_eq!(
        transform_rerank_request(&params(
            r#"{"query": "q", "texts": ["d"], "truncate": true, "raw_scores": true, "truncation_direction": "Left"}"#
        ))
        .unwrap(),
        params(
            r#"{"query": "q", "texts": ["d"], "raw_scores": true, "truncate": true, "truncation_direction": "Left"}"#
        )
    );
}

#[test]
fn request_body_requires_query() {
    assert!(matches!(
        transform_rerank_request(&params(r#"{"texts": ["d"]}"#)),
        Err(HuggingfaceRerankError::MissingQuery)
    ));
}

#[test]
fn request_body_requires_texts() {
    assert!(matches!(
        transform_rerank_request(&params(r#"{"query": "q"}"#)),
        Err(HuggingfaceRerankError::MissingTexts)
    ));
}

#[test]
fn request_body_keeps_a_null_query_as_present() {
    assert!(transform_rerank_request(&params(r#"{"query": null, "texts": []}"#)).is_ok());
}

#[test]
fn request_headers_start_from_json_defaults() {
    let resolved = validate_environment(&Headers::new(), None);
    assert_eq!(
        resolved,
        Headers::from([
            ("accept".to_owned(), "application/json".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ])
    );
}

#[test]
fn request_headers_add_a_bearer_for_a_key() {
    let key = SecretString::from("sk-hf");
    assert_eq!(
        validate_environment(&Headers::new(), Some(&key)).get("Authorization"),
        Some(&"Bearer sk-hf".to_owned())
    );
}

#[test]
fn request_headers_bear_an_empty_key_too() {
    let key = SecretString::from("");
    assert_eq!(
        validate_environment(&Headers::new(), Some(&key)).get("Authorization"),
        Some(&"Bearer ".to_owned())
    );
}

#[test]
fn request_headers_let_the_caller_override_every_default() {
    let caller = headers(r#"{"accept": "text/plain", "content-type": "text/xml"}"#);
    assert_eq!(
        validate_environment(&caller, None),
        Headers::from([
            ("accept".to_owned(), "text/plain".to_owned()),
            ("content-type".to_owned(), "text/xml".to_owned()),
        ])
    );
}

#[test]
fn request_headers_keep_the_caller_authorization() {
    let caller = headers(r#"{"Authorization": "Bearer caller"}"#);
    let key = SecretString::from("sk-hf");
    assert_eq!(
        validate_environment(&caller, Some(&key)).get("Authorization"),
        Some(&"Bearer caller".to_owned())
    );
}

#[test]
fn response_projects_index_and_score() {
    let response = success(
        r#"[{"index": 1, "score": 0.25}, {"index": 0, "score": 0.5}]"#,
        r#"{"query": "q"}"#,
        &[Some(3), Some(4)],
    );
    let projected = results(&response);
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].index, 1);
    assert_eq!(projected[0].relevance_score, score(0.25));
    assert_eq!(projected[1].index, 0);
    assert_eq!(projected[1].relevance_score, score(0.5));
}

#[test]
fn response_keeps_only_items_with_an_index_and_a_score() {
    let response = success(
        r#"[{"index": 0}, {"score": 0.5}, {"index": 2, "score": 0.1}, {"index": null, "score": 0.2}, {"index": 3, "score": null}]"#,
        r#"{"query": "q"}"#,
        &[Some(3), Some(4)],
    );
    let projected = results(&response);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].index, 2);
}

#[test]
fn response_id_comes_from_the_id_source() {
    let response =
        run_response(r#"[]"#, Some(200), r#"{}"#, &[Some(1), Some(2)], "the-id").unwrap();
    assert_eq!(response.id.as_deref(), Some("the-id"));
}

#[test]
fn response_meta_carries_the_version_billed_units_and_tokens() {
    let response = success(r#"[]"#, r#"{"query": "q"}"#, &[Some(7), Some(11)]);
    let response_meta = meta(&response);
    assert_eq!(
        response_meta.api_version,
        Some(Some(indexmap::IndexMap::from([(
            "version".to_owned(),
            Json::Text("1.0".to_owned())
        )])))
    );
    assert_eq!(
        response_meta
            .billed_units
            .unwrap_or_default()
            .unwrap_or_default()
            .search_units,
        Some(Some(1))
    );
    assert_eq!(tokens(&response), (11, 7));
}

#[test]
fn response_adds_documents_when_return_text_is_set() {
    let response = success(
        r#"[{"index": 0, "score": 0.5, "text": "provider"}]"#,
        r#"{"query": "q", "texts": ["first"], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, Some(document("provider")));
}

#[test]
fn response_adds_documents_when_return_documents_is_set() {
    let response = success(
        r#"[{"index": 0, "score": 0.5, "text": "provider"}]"#,
        r#"{"query": "q", "texts": ["first"], "return_documents": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, Some(document("provider")));
}

#[rstest]
#[case::return_text(r#"{"query": "q", "texts": ["first"], "return_text": false}"#)]
#[case::return_documents(r#"{"query": "q", "texts": ["first"], "return_documents": false}"#)]
#[case::absent(r#"{"query": "q", "texts": ["first"]}"#)]
fn response_adds_no_documents_without_a_flag(#[case] request_data: &str) {
    let response = success(
        r#"[{"index": 0, "score": 0.5, "text": "provider"}]"#,
        request_data,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, None);
}

#[test]
fn response_falls_back_to_the_request_document() {
    let response = success(
        r#"[{"index": 1, "score": 0.5}]"#,
        r#"{"query": "q", "texts": ["first", "second"], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, Some(document("second")));
}

#[test]
fn response_prefers_the_item_text_over_the_request_document() {
    let response = success(
        r#"[{"index": 0, "score": 0.5, "text": "provider"}]"#,
        r#"{"query": "q", "texts": ["request"], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, Some(document("provider")));
}

#[test]
fn response_uses_an_object_request_document() {
    let response = success(
        r#"[{"index": 1, "score": 0.5}]"#,
        r#"{"query": "q", "texts": ["first", {"text": "second"}], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, Some(document("second")));
}

#[test]
fn response_skips_a_document_index_past_the_request_documents() {
    let response = success(
        r#"[{"index": 2, "score": 0.5}]"#,
        r#"{"query": "q", "texts": ["first"], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, None);
}

#[test]
fn response_skips_a_negative_document_index() {
    let response = success(
        r#"[{"index": -1, "score": 0.5}]"#,
        r#"{"query": "q", "texts": ["first"], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, None);
}

#[test]
fn response_keeps_an_object_document_without_text_out() {
    let response = success(
        r#"[{"index": 0, "score": 0.5}]"#,
        r#"{"query": "q", "texts": [{"other": "x"}], "return_text": true}"#,
        &[Some(1), Some(2)],
    );
    assert_eq!(results(&response)[0].document, None);
}

#[test]
fn response_counts_the_parsed_list_and_the_query_with_documents() {
    let request_data = params(r#"{"query": "q", "texts": ["a", {"text": "b"}]}"#);
    let counter = RecordedCounter::new(&[Some(5), Some(9)]);
    let request = ResponseRequest {
        model: MODEL,
        status_code: Some(200),
        body: br#"[{"index": 0, "score": 0.5}]"#,
        request_data: &request_data,
    };
    let response = transform_rerank_response(&request, &counter, &FixedId("id")).unwrap();
    assert_eq!(tokens(&response), (9, 5));
    assert_eq!(
        counter.seen(),
        vec![
            (MODEL.to_owned(), r#"[{"index":0,"score":0.5}]"#.to_owned()),
            (MODEL.to_owned(), "q a b ".to_owned()),
        ]
    );
}

#[test]
fn response_output_fallback_is_ten_per_item() {
    let response = success(
        r#"[{"index": 0, "score": 0.5}, {"index": 1, "score": 0.25}]"#,
        r#"{"query": "q"}"#,
        &[None, Some(9)],
    );
    assert_eq!(tokens(&response), (4, 20));
}

#[test]
fn response_output_fallback_is_ten_for_an_empty_list() {
    let response = success(r#"[]"#, r#"{"query": ""}"#, &[None, Some(9)]);
    assert_eq!(tokens(&response), (0, 10));
}

#[test]
fn response_input_fallback_uses_the_query_alone() {
    let response = success(r#"[]"#, r#"{"query": "query"}"#, &[None, Some(9)]);
    assert_eq!(tokens(&response), (20, 10));
}

#[test]
fn response_input_fallback_uses_the_full_query_and_documents() {
    let response = success(
        r#"[{"index": 0, "score": 0.5}]"#,
        r#"{"query": "q", "texts": ["ab"]}"#,
        &[Some(3), None],
    );
    assert_eq!(tokens(&response), (20, 3));
}

#[test]
fn response_malformed_body_is_a_huggingface_error() {
    let error = run_response("not json", Some(503), r#"{}"#, &[], "id").unwrap_err();
    match error {
        HuggingfaceRerankError::Huggingface(source) => {
            assert_eq!(source.status_code, 503);
            assert_eq!(source.message, "not json");
            assert!(source.headers.is_empty());
        }
        HuggingfaceRerankError::MissingQuery
        | HuggingfaceRerankError::MissingTexts
        | HuggingfaceRerankError::ResponseTooLarge { .. }
        | HuggingfaceRerankError::ResponseShape
        | HuggingfaceRerankError::ResponseItem { .. }
        | HuggingfaceRerankError::RequestField { .. }
        | HuggingfaceRerankError::DocumentText { .. }
        | HuggingfaceRerankError::TokenOverflow => panic!("unexpected error"),
    }
}

#[test]
fn response_malformed_body_without_a_status_reports_500() {
    let error = run_response("not json", None, r#"{}"#, &[], "id").unwrap_err();
    assert!(matches!(
        error,
        HuggingfaceRerankError::Huggingface(source) if source.status_code == 500
    ));
}

#[test]
fn response_non_list_body_is_a_shape_error() {
    assert!(matches!(
        run_response(r#"{"index": 0}"#, Some(200), r#"{}"#, &[], "id"),
        Err(HuggingfaceRerankError::ResponseShape)
    ));
}

#[test]
fn response_non_object_item_is_an_item_error() {
    assert!(matches!(
        run_response(r#"[1]"#, Some(200), r#"{}"#, &[], "id"),
        Err(HuggingfaceRerankError::ResponseItem { position: 0, .. })
    ));
}

#[test]
fn response_oversized_body_is_rejected() {
    let body = vec![b' '; MAX_RESPONSE_BYTES + 1];
    assert!(matches!(
        decode_rerank_response(&body, Some(200)),
        Err(HuggingfaceRerankError::ResponseTooLarge { .. })
    ));
}

#[test]
fn response_body_at_the_limit_is_decoded() {
    let mut body = vec![b' '; MAX_RESPONSE_BYTES];
    body[0] = b'[';
    body[MAX_RESPONSE_BYTES - 1] = b']';
    assert!(decode_rerank_response(&body, Some(200)).is_ok());
}

#[test]
fn response_document_text_must_be_a_string() {
    assert!(matches!(
        run_response(
            r#"[{"index": 0, "score": 0.5}]"#,
            Some(200),
            r#"{"query": "q", "texts": [{"text": 5}]}"#,
            &[Some(1), Some(2)],
            "id"
        ),
        Err(HuggingfaceRerankError::DocumentText { position: 0 })
    ));
}

#[test]
fn response_request_query_must_be_a_string() {
    assert!(matches!(
        run_response(r#"[]"#, Some(200), r#"{"query": 5}"#, &[], "id"),
        Err(HuggingfaceRerankError::RequestField { field: "query" })
    ));
}

#[test]
fn response_request_texts_must_be_a_list() {
    assert!(matches!(
        run_response(r#"[]"#, Some(200), r#"{"texts": "x"}"#, &[], "id"),
        Err(HuggingfaceRerankError::RequestField { field: "texts" })
    ));
}

#[test]
fn response_decoded_item_keeps_its_text() {
    let items: Vec<RerankResponseItem> =
        decode_rerank_response(br#"[{"index": 3, "score": 1, "text": "hi"}]"#, Some(200)).unwrap();
    assert_eq!(items[0].index, Some(3));
    assert_eq!(items[0].text.as_deref(), Some("hi"));
    assert_eq!(items[0].score, Some(score(1.0)));
}

#[rstest]
fn replays_the_rerank_response_corpus(
    #[files("fuzz/corpus/huggingface_rerank_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("corpus file");
    let _decoded = decode_rerank_response(&bytes, Some(200));
}
