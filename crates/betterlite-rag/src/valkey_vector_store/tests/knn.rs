use betterlite_provider::api::vector_store_contracts::config::SearchQuery;
use rstest::rstest;

use super::fakes::{empty_params, params};
use crate::valkey_vector_store::error::ValkeyError;
use crate::valkey_vector_store::query::{
    DEFAULT_EMBEDDING_FIELD, DEFAULT_MAX_NUM_RESULTS, DEFAULT_TEXT_FIELD, ValkeyKnnQuery,
    knn_limit, knn_query, query_text,
};

#[test]
fn query_text_uses_a_string_even_when_it_is_empty() {
    assert_eq!(
        query_text(&SearchQuery::Text(String::new())).expect("the empty string"),
        ""
    );
    assert_eq!(
        query_text(&SearchQuery::Text("find me".to_owned())).expect("the string"),
        "find me"
    );
}

#[test]
fn query_text_joins_a_list_with_one_space() {
    let query = SearchQuery::Sequence(vec!["one".to_owned(), "two".to_owned()]);
    assert_eq!(query_text(&query).expect("the list"), "one two");
}

#[test]
fn query_text_rejects_an_empty_list() {
    let error = query_text(&SearchQuery::Sequence(Vec::new())).expect_err("the empty list");
    assert_eq!(error.to_string(), "query must not be empty");
}

#[test]
fn knn_query_matches_litellm_defaults() {
    let query =
        knn_query(&empty_params(), DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD).expect("the query");
    assert_eq!(
        query,
        ValkeyKnnQuery {
            expression: "*=>[KNN 10 @embedding $vec AS vector_distance]".to_owned(),
            return_fields: vec!["text".to_owned(), "vector_distance".to_owned()],
            offset: 0,
            count: 10,
            dialect: 2,
        }
    );
}

#[test]
fn knn_query_uses_the_configured_fields_and_limit() {
    let optional = params(r#"{"max_num_results":3}"#);
    let query = knn_query(&optional, "vector", "content").expect("the query");
    assert_eq!(
        query,
        ValkeyKnnQuery {
            expression: "*=>[KNN 3 @vector $vec AS vector_distance]".to_owned(),
            return_fields: vec!["content".to_owned(), "vector_distance".to_owned()],
            offset: 0,
            count: 3,
            dialect: 2,
        }
    );
}

#[rstest]
#[case::absent("{}", DEFAULT_MAX_NUM_RESULTS)]
#[case::null(r#"{"max_num_results":null}"#, DEFAULT_MAX_NUM_RESULTS)]
#[case::minimum(r#"{"max_num_results":1}"#, 1)]
#[case::maximum(r#"{"max_num_results":50}"#, 50)]
fn knn_limit_accepts(#[case] source: &str, #[case] expected: i64) {
    assert_eq!(knn_limit(&params(source)).expect("the limit"), expected);
}

#[rstest]
#[case::zero(r#"{"max_num_results":0}"#, 0)]
#[case::above_maximum(r#"{"max_num_results":51}"#, 51)]
#[case::negative(r#"{"max_num_results":-2}"#, -2)]
fn knn_limit_rejects_out_of_range(#[case] source: &str, #[case] value: i64) {
    let error = knn_limit(&params(source)).expect_err("the out of range limit");
    assert_eq!(
        error.to_string(),
        format!("max_num_results must be between 1 and 50, got {value}")
    );
}

#[test]
fn knn_limit_rejects_a_wrong_type() {
    let error = knn_limit(&params(r#"{"max_num_results":"10"}"#)).expect_err("the text limit");
    assert_eq!(
        error.to_string(),
        "the max_num_results value has the wrong type: expected an integer"
    );
}

#[rstest]
#[case::object("{}")]
#[case::non_empty(r#"{"key":"tag","value":"x"}"#)]
fn knn_query_rejects_filters(#[case] filters: &str) {
    let optional = params(&format!(r#"{{"filters":{filters}}}"#));
    let error =
        knn_query(&optional, DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD).expect_err("the filters");
    assert_eq!(
        error.to_string(),
        "Valkey vector store does not support the filters parameter yet"
    );
}

#[test]
fn knn_query_accepts_null_filters() {
    let optional = params(r#"{"filters":null}"#);
    let query = knn_query(&optional, DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD)
        .expect("the null filters");
    assert_eq!(query.count, 10);
}

#[test]
fn knn_query_checks_filters_before_the_limit() {
    let optional = params(r#"{"filters":{},"max_num_results":51}"#);
    let error =
        knn_query(&optional, DEFAULT_EMBEDDING_FIELD, DEFAULT_TEXT_FIELD).expect_err("the filters");
    assert!(matches!(error, ValkeyError::FiltersUnsupported));
}
