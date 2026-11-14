use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreSearchResponse, VectorStoreSearchResult,
};
use rstest::rstest;

use super::support::{json, reply_headers};
use crate::s3_vectors_search::error::S3VectorsSearchError;
use crate::s3_vectors_search::response::{SEARCH_RESULTS_OBJECT, decode_s3_vectors_search};

fn decode(body: &str, query_text: Option<&str>) -> VectorStoreSearchResponse {
    decode_s3_vectors_search(200, &reply_headers(), body.as_bytes(), query_text).unwrap()
}

fn wire_score(result: &VectorStoreSearchResult) -> Option<String> {
    let Ok(text) = serde_json::to_string(result) else {
        panic!("the result cannot be serialized");
    };
    let Json::Object(fields) = json(&text) else {
        panic!("the result is not an object");
    };
    let Some(score) = fields.get("score") else {
        panic!("the result has no score key");
    };
    if score == &Json::Null {
        return None;
    }
    let Json::Number(number) = score else {
        panic!("the score is not a number");
    };
    Some(number.to_string())
}

fn one_score(distance: &str) -> Option<String> {
    let body = format!(
        r#"{{"vectors": [{{"distance": {distance}, "metadata": {{"source_text": "t"}}}}]}}"#
    );
    let response = decode(&body, None);
    let results = response.data?;
    wire_score(results.first()?)
}

#[test]
fn a_hit_becomes_one_text_result() {
    let body = r#"{"vectors": [{"key": "k1", "distance": 0.25, "metadata":
        {"source_text": "alpha", "chunk_index": "4", "filename": "a.txt", "extra": [1, 2]}}]}"#;
    let response = decode(body, Some("the query"));
    assert_eq!(response.object.as_deref(), Some(SEARCH_RESULTS_OBJECT));
    assert_eq!(response.search_query.as_deref(), Some("the query"));
    let results = response.data.unwrap();
    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.score.as_ref().unwrap().0.to_string(), "0.75");
    assert_eq!(result.file_id.as_deref(), Some("s3-vectors-chunk-4"));
    assert_eq!(result.filename.as_deref(), Some("a.txt"));
    let content = result.content.as_ref().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].text.as_deref(), Some("alpha"));
    assert_eq!(content[0].type_name.as_deref(), Some("text"));
    let attributes: Json = serde_json::from_value(result.attributes.clone().unwrap()).unwrap();
    let metadata = r#"{"source_text":"alpha","chunk_index":"4","filename":"a.txt","extra":[1,2]}"#;
    assert_eq!(attributes, json(metadata));
}

#[rstest]
#[case::missing(r#"{"vectors": [{"metadata": {}}]}"#)]
#[case::null_metadata(r#"{"vectors": [{"metadata": null}]}"#)]
#[case::no_metadata(r#"{"vectors": [{"distance": 0.1}]}"#)]
#[case::null_text(r#"{"vectors": [{"metadata": {"source_text": null}}]}"#)]
#[case::empty_text(r#"{"vectors": [{"metadata": {"source_text": ""}}]}"#)]
fn an_item_without_source_text_is_skipped(#[case] body: &str) {
    assert_eq!(decode(body, None).data, Some(Vec::new()));
}

#[test]
fn skipped_items_leave_the_others_in_order() {
    let body = r#"{"vectors": [
        {"metadata": {"source_text": "first"}},
        {"metadata": {"source_text": ""}},
        {"metadata": {"source_text": "third", "chunk_index": 2}}]}"#;
    let results = decode(body, None).data.unwrap();
    let ids: Vec<&str> = results
        .iter()
        .map(|result| result.file_id.as_deref().unwrap())
        .collect();
    assert_eq!(ids, ["s3-vectors-chunk-0", "s3-vectors-chunk-2"]);
}

#[rstest]
#[case::default(r#"{"source_text": "t"}"#, "s3-vectors-chunk-0", "document-0")]
#[case::null(
    r#"{"source_text": "t", "chunk_index": null}"#,
    "s3-vectors-chunk-0",
    "document-0"
)]
#[case::text(
    r#"{"source_text": "t", "chunk_index": "9"}"#,
    "s3-vectors-chunk-9",
    "document-9"
)]
#[case::number(
    r#"{"source_text": "t", "chunk_index": 12}"#,
    "s3-vectors-chunk-12",
    "document-12"
)]
#[case::named(
    r#"{"source_text": "t", "chunk_index": 3, "filename": "n.pdf"}"#,
    "s3-vectors-chunk-3",
    "n.pdf"
)]
#[case::null_filename(
    r#"{"source_text": "t", "chunk_index": 3, "filename": null}"#,
    "s3-vectors-chunk-3",
    "document-3"
)]
#[case::empty_filename_is_kept(
    r#"{"source_text": "t", "chunk_index": 3, "filename": ""}"#,
    "s3-vectors-chunk-3",
    ""
)]
fn file_id_and_filename_follow_the_chunk_index(
    #[case] metadata: &str,
    #[case] file_id: &str,
    #[case] filename: &str,
) {
    let body = format!(r#"{{"vectors": [{{"metadata": {metadata}}}]}}"#);
    let results = decode(&body, None).data.unwrap();
    assert_eq!(results[0].file_id.as_deref(), Some(file_id));
    assert_eq!(results[0].filename.as_deref(), Some(filename));
}

#[rstest]
#[case::exact("0.25", Some("0.75"))]
#[case::zero_distance("0", Some("1.0"))]
#[case::one_distance("1", Some("0.0"))]
#[case::just_inside_one("0.9", Some("0.09999999999999998"))]
#[case::float_remainder("0.7", Some("0.30000000000000004"))]
#[case::above_one_is_zero("1.5", Some("0.0"))]
#[case::negative_is_one("-0.5", Some("1.0"))]
#[case::null("null", None)]
fn the_score_is_one_minus_distance_clamped(#[case] distance: &str, #[case] expected: Option<&str>) {
    assert_eq!(one_score(distance).as_deref(), expected);
}

#[test]
fn a_float32_distance_next_to_one_keeps_its_tiny_score() {
    let score = one_score("0.99999994").unwrap();
    let value: f64 = score.parse().unwrap();
    assert!((5e-8_f64..7e-8_f64).contains(&value), "{score}");
}

#[test]
fn an_absent_distance_gives_no_score() {
    let response = decode(r#"{"vectors": [{"metadata": {"source_text": "t"}}]}"#, None);
    let results = response.data.unwrap();
    assert_eq!(wire_score(&results[0]), None);
}

#[rstest]
#[case::missing("{}")]
#[case::null_vectors(r#"{"vectors": null}"#)]
#[case::empty_vectors(r#"{"vectors": []}"#)]
fn no_vectors_gives_an_empty_page(#[case] body: &str) {
    let response = decode(body, None);
    assert_eq!(response.data, Some(Vec::new()));
    assert_eq!(response.object.as_deref(), Some(SEARCH_RESULTS_OBJECT));
}

#[test]
fn the_search_query_is_empty_when_the_request_step_saw_none() {
    assert_eq!(decode("{}", None).search_query.as_deref(), Some(""));
    assert_eq!(decode("{}", Some("")).search_query.as_deref(), Some(""));
}

#[rstest]
#[case::not_json("not json")]
#[case::empty("")]
#[case::top_level_array("[]")]
#[case::vectors_string(r#"{"vectors": "x"}"#)]
#[case::vectors_object(r#"{"vectors": {}}"#)]
#[case::item_not_object(r#"{"vectors": [1]}"#)]
#[case::metadata_not_object(r#"{"vectors": [{"metadata": "x"}]}"#)]
#[case::source_text_number(r#"{"vectors": [{"metadata": {"source_text": 5}}]}"#)]
#[case::chunk_index_bool(
    r#"{"vectors": [{"metadata": {"source_text": "t", "chunk_index": true}}]}"#
)]
#[case::filename_number(r#"{"vectors": [{"metadata": {"source_text": "t", "filename": 7}}]}"#)]
#[case::distance_text(r#"{"vectors": [{"distance": "0.1", "metadata": {"source_text": "t"}}]}"#)]
fn an_unreadable_body_is_a_provider_error_with_status_and_headers(#[case] body: &str) {
    let error = decode_s3_vectors_search(502, &reply_headers(), body.as_bytes(), None).unwrap_err();
    let S3VectorsSearchError::Provider(provider) = error else {
        panic!("expected a provider error");
    };
    assert_eq!(provider.status_code, 502);
    assert!(!provider.message.is_empty());
    assert_eq!(
        provider.headers.get("x-request-id"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert_eq!(provider.headers.len(), 1);
}
