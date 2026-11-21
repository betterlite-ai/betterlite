#![cfg(test)]

use betterlite_error::error::{ErrorClass, class_message};
use betterlite_openai::search::provider::SearchQuery;
use rstest::rstest;

use super::error::SearchCallError;
use super::query::{QueryError, search_query};
use super::support::{Harness, Recorder, Registry, json, request, status_of};

#[rstest]
#[case::text(r#""rust""#, SearchQuery::Single("rust".to_owned()))]
#[case::empty_text(r#""""#, SearchQuery::Single(String::new()))]
#[case::list(r#"["rust","go"]"#, SearchQuery::Multiple(vec!["rust".to_owned(), "go".to_owned()]))]
#[case::empty_list("[]", SearchQuery::Multiple(Vec::new()))]
fn query_validation_accepts_text_and_text_lists(
    #[case] query: &str,
    #[case] expected: SearchQuery,
) {
    assert_eq!(search_query(&json(query)), Ok(expected));
}

#[rstest]
#[case::null("null", "null")]
#[case::boolean("true", "a boolean")]
#[case::integer("3", "a number")]
#[case::float("1.5", "a number")]
#[case::object(r#"{"q":"rust"}"#, "an object")]
fn query_validation_rejects_other_kinds(#[case] query: &str, #[case] found: &'static str) {
    let rejected = search_query(&json(query)).unwrap_err();
    assert_eq!(rejected, QueryError::Kind { found });
    assert_eq!(
        rejected.to_string(),
        format!("query must be a string or list of strings, got {found}")
    );
}

#[rstest]
#[case::number_item(r#"["rust",1]"#)]
#[case::null_item(r#"["rust",null]"#)]
#[case::boolean_item("[false]")]
#[case::nested_list(r#"[["rust"]]"#)]
#[case::object_item(r#"[{"q":"rust"}]"#)]
fn query_validation_rejects_lists_with_other_items(#[case] query: &str) {
    let rejected = search_query(&json(query)).unwrap_err();
    assert_eq!(rejected, QueryError::Item);
    assert_eq!(
        rejected.to_string(),
        "All items in query list must be strings"
    );
}

#[tokio::test]
async fn query_validation_runs_before_the_provider_lookup() {
    let registry = Registry::new(Recorder::default());
    let mut unknown = request(r#"["rust",1]"#, "http://127.0.0.1:9");
    unknown.search_provider = "nowhere".to_owned();
    let mut harness = Harness::new();
    let failure = harness.search(&unknown, &registry).await.unwrap_err();
    assert!(matches!(
        failure.error,
        SearchCallError::Query(QueryError::Item)
    ));
    assert!(registry.asked().is_empty());
    assert!(failure.response_headers.is_empty());
    assert_eq!(
        failure.to_string(),
        "All items in query list must be strings"
    );
}

#[tokio::test]
async fn query_validation_failure_is_a_bad_request() {
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let failure = harness
        .search(&request("42", "http://127.0.0.1:9"), &registry)
        .await
        .unwrap_err();
    assert!(matches!(failure.class, ErrorClass::BadRequest(_)));
    assert_eq!(status_of(&failure.class), 400);
    let metadata = failure.class.metadata();
    assert_eq!(metadata.model.as_deref(), Some("brave/search"));
    assert_eq!(metadata.provider.as_deref(), Some("brave"));
    assert_eq!(
        class_message(&failure.class),
        "query must be a string or list of strings, got a number"
    );
    assert_eq!(harness.state.model, "unset");
}
