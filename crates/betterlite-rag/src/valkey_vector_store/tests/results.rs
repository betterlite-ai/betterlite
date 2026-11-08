use std::str::FromStr as _;

use betterlite_openai::api::vector_store_records::records::score::Score;
use rstest::rstest;
use rust_decimal::Decimal;

use super::fakes::document;
use crate::valkey_vector_store::reply::search_response;

fn score(value: &str) -> Score {
    Score(Decimal::from_str(value).expect("a score fixture"))
}

#[test]
fn results_score_is_one_minus_the_distance() {
    let documents = vec![document(
        "doc",
        &[("text", "hello"), ("vector_distance", "0.25")],
    )];
    let response = search_response(&documents, "find", "text").expect("the response");
    assert_eq!(
        response.object.as_deref(),
        Some("vector_store.search_results.page")
    );
    assert_eq!(response.search_query.as_deref(), Some("find"));
    let data = response.data.expect("the data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].score, Some(score("0.75")));
    assert_eq!(data[0].file_id.as_deref(), Some("doc"));
    assert_eq!(data[0].filename.as_deref(), Some("doc"));
    let content = data[0].content.clone().expect("the content");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0].text.as_deref(), Some("hello"));
    assert_eq!(content[0].type_name.as_deref(), Some("text"));
}

#[test]
fn results_use_an_empty_string_for_a_missing_text_field() {
    let documents = vec![document("doc", &[("vector_distance", "0.5")])];
    let response = search_response(&documents, "find", "text").expect("the response");
    let data = response.data.expect("the data");
    let content = data[0].content.clone().expect("the content");
    assert_eq!(content[0].text.as_deref(), Some(""));
}

#[test]
fn results_read_the_configured_text_field() {
    let documents = vec![document(
        "doc",
        &[
            ("text", "wrong"),
            ("content", "right"),
            ("vector_distance", "0.5"),
        ],
    )];
    let response = search_response(&documents, "find", "content").expect("the response");
    let data = response.data.expect("the data");
    let content = data[0].content.clone().expect("the content");
    assert_eq!(content[0].text.as_deref(), Some("right"));
}

#[test]
fn results_sort_by_score_descending() {
    let documents = vec![
        document("low", &[("vector_distance", "0.9")]),
        document("high", &[("vector_distance", "0.1")]),
        document("middle", &[("vector_distance", "0.5")]),
    ];
    let response = search_response(&documents, "find", "text").expect("the response");
    let data = response.data.expect("the data");
    let order: Vec<&str> = data
        .iter()
        .filter_map(|result| result.file_id.as_deref())
        .collect();
    assert_eq!(order, ["high", "middle", "low"]);
}

#[test]
fn results_keep_ties_in_input_order() {
    let documents = vec![
        document("first", &[("vector_distance", "0.25")]),
        document("second", &[("vector_distance", "0.25")]),
        document("third", &[("vector_distance", "0.1")]),
    ];
    let response = search_response(&documents, "find", "text").expect("the response");
    let data = response.data.expect("the data");
    let order: Vec<&str> = data
        .iter()
        .filter_map(|result| result.file_id.as_deref())
        .collect();
    assert_eq!(order, ["third", "first", "second"]);
}

#[test]
fn results_reject_a_document_without_a_distance() {
    let documents = vec![document("doc", &[("text", "x")])];
    let error = search_response(&documents, "find", "text").expect_err("the missing distance");
    assert_eq!(
        error.to_string(),
        "the search result for document 'doc' has no vector_distance field"
    );
}

#[rstest]
#[case::not_a_number("close")]
#[case::infinity("inf")]
#[case::not_a_number_yet("NaN")]
fn results_reject_an_invalid_distance(#[case] distance: &str) {
    let documents = vec![document("doc", &[("vector_distance", distance)])];
    let error = search_response(&documents, "find", "text").expect_err("the invalid distance");
    assert_eq!(
        error.to_string(),
        format!(
            "the search result for document 'doc' has an invalid vector_distance: '{distance}'"
        )
    );
}

#[test]
fn results_write_the_search_page_shape() {
    let documents = vec![document(
        "doc-a",
        &[("text", "hello"), ("vector_distance", "0.25")],
    )];
    let response = search_response(&documents, "find me", "text").expect("the response");
    let value = serde_json::to_value(&response).expect("the response json");
    assert_eq!(value["object"], "vector_store.search_results.page");
    assert_eq!(value["search_query"], "find me");
    assert_eq!(value["data"][0]["file_id"], "doc-a");
    assert_eq!(value["data"][0]["filename"], "doc-a");
    assert_eq!(value["data"][0]["content"][0]["text"], "hello");
    assert_eq!(value["data"][0]["content"][0]["type"], "text");
    assert_eq!(value["data"][0]["score"], serde_json::json!(0.75_f64));
}

#[test]
fn results_write_the_python_float_digits() {
    let documents = vec![document(
        "doc",
        &[("vector_distance", "0.051030516624450684")],
    )];
    let response = search_response(&documents, "find", "text").expect("the response");
    let value = serde_json::to_value(&response).expect("the response json");
    assert_eq!(
        value["data"][0]["score"],
        serde_json::json!(0.948_969_483_375_549_3_f64)
    );
}

#[rstest]
#[case::just_under_one("0.99999994039535522", true)]
#[case::just_over_one("1.0000000596046448", false)]
fn results_keep_scores_next_to_zero(#[case] distance: &str, #[case] positive: bool) {
    let documents = vec![document("doc", &[("vector_distance", distance)])];
    let response = search_response(&documents, "find", "text").expect("the response");
    let data = response.data.expect("the data");
    let score = data[0].score.as_ref().expect("the score").0;
    assert_eq!(score.is_sign_positive(), positive);
    assert!(!score.is_zero());
}

#[test]
fn results_of_an_empty_reply_are_empty() {
    let response = search_response(&[], "find", "text").expect("the response");
    assert_eq!(response.data, Some(Vec::new()));
}
