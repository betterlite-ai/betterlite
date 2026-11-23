#![cfg(test)]

use rstest::rstest;

use super::is_index_lifecycle_request;

#[rstest]
#[case::post("POST")]
#[case::put("PUT")]
#[case::delete("DELETE")]
#[case::patch("PATCH")]
fn the_index_itself_is_a_lifecycle_request(#[case] method: &str) {
    assert!(is_index_lifecycle_request(
        method,
        "/azure_ai/indexes/my-index",
        "my-index"
    ));
}

#[rstest]
#[case::post("POST")]
#[case::put("PUT")]
#[case::delete("DELETE")]
#[case::patch("PATCH")]
fn a_query_after_the_index_name_is_still_the_index(#[case] method: &str) {
    assert!(is_index_lifecycle_request(
        method,
        "/azure_ai/indexes/my-index?x=1",
        "my-index"
    ));
}

#[rstest]
#[case::docs_search("POST", "/azure_ai/indexes/my-index/docs/search")]
#[case::docs_without_slash("POST", "/azure_ai/indexes/my-index/docsanything")]
#[case::docs_itself("DELETE", "/azure_ai/indexes/my-index/docs")]
#[case::docs_member("PATCH", "/azure_ai/indexes/my-index/docs/1")]
fn a_document_operation_is_not_an_index_lifecycle_request(
    #[case] method: &str,
    #[case] path: &str,
) {
    assert!(!is_index_lifecycle_request(method, path, "my-index"));
}

#[rstest]
#[case::delete("DELETE", "/azure_ai/indexes/my-index/")]
#[case::post("POST", "/azure_ai/indexes/my-index/")]
#[case::other_segment("PUT", "/azure_ai/indexes/my-index/other")]
#[case::later_occurrence("DELETE", "/indexes/my-index/x/indexes/my-index")]
fn a_suffix_that_is_neither_the_index_nor_docs_is_no_lifecycle_request(
    #[case] method: &str,
    #[case] path: &str,
) {
    assert!(!is_index_lifecycle_request(method, path, "my-index"));
}

#[rstest]
#[case::get("GET", "/azure_ai/indexes/my-index")]
#[case::head("HEAD", "/azure_ai/indexes/my-index")]
#[case::options("OPTIONS", "/azure_ai/indexes/my-index")]
#[case::get_service("GET", "/v1/indexes")]
#[case::get_indexes("GET", "/indexes")]
fn a_read_method_never_reaches_the_index_rules(#[case] method: &str, #[case] path: &str) {
    assert!(!is_index_lifecycle_request(method, path, "my-index"));
}

#[rstest]
#[case::root("POST", "/indexes")]
#[case::trailing_slash("POST", "/indexes/")]
#[case::query("POST", "/indexes/?x=1")]
#[case::double_slash("POST", "/indexes//")]
#[case::provider_prefix("POST", "/azure_ai/indexes")]
#[case::many_slashes("POST", "/azure_ai/indexes///")]
#[case::other_prefix("POST", "/prefix/indexes")]
#[case::with_query("POST", "/v1/indexes?x=1")]
#[case::after_a_name("POST", "/indexes/my-index/indexes")]
fn a_post_that_ends_at_the_service_indexes_is_a_create(#[case] method: &str, #[case] path: &str) {
    assert!(is_index_lifecycle_request(method, path, "my-index"));
}

#[rstest]
#[case::delete("DELETE", "/indexes")]
#[case::put("PUT", "/azure_ai/indexes")]
#[case::patch("PATCH", "/indexes/")]
fn only_a_post_creates_an_index_at_the_service_level(#[case] method: &str, #[case] path: &str) {
    assert!(!is_index_lifecycle_request(method, path, "my-index"));
}

#[rstest]
#[case::dot_in_name("DELETE", "/azure_ai/indexes/axb", "a.b", false)]
#[case::literal_dot("DELETE", "/azure_ai/indexes/a.b", "a.b", true)]
#[case::longer_name("DELETE", "/azure_ai/indexes/name2", "name", false)]
#[case::later_name("DELETE", "/azure_ai/indexes/name2/indexes/name", "name", true)]
#[case::first_name_wins("DELETE", "/azure_ai/indexes/name/indexes/name2", "name", false)]
#[case::first_inside_prefix("DELETE", "/indexes/name/indexes/name", "name", false)]
#[case::query_before_later_name("DELETE", "/indexes/name?x=1/indexes/name", "name", true)]
fn the_index_name_is_a_literal_with_a_lookahead(
    #[case] method: &str,
    #[case] path: &str,
    #[case] index_name: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        is_index_lifecycle_request(method, path, index_name),
        expected
    );
}

#[rstest]
#[case::overlapping_occurrence("DELETE", "/indexes/indexes/index", "index", true)]
#[case::overlapping_followed_by_another_char("DELETE", "/indexes/indexes/index2", "index", false)]
#[case::overlapping_before_the_query("DELETE", "/indexes/indexes/index?x=1", "index", true)]
fn an_occurrence_overlapping_a_rejected_start_is_still_found(
    #[case] method: &str,
    #[case] path: &str,
    #[case] index_name: &str,
    #[case] expected: bool,
) {
    assert_eq!(
        is_index_lifecycle_request(method, path, index_name),
        expected
    );
}

#[rstest]
#[case::unrelated("DELETE", "/v1/vector_stores/vs-1")]
#[case::post_unrelated("POST", "/v1/vector_stores/vs-1")]
#[case::empty("POST", "")]
#[case::query_only("POST", "?x=1")]
#[case::no_leading_slash("POST", "indexes")]
fn a_path_without_the_index_route_is_not_a_lifecycle_request(
    #[case] method: &str,
    #[case] path: &str,
) {
    assert!(!is_index_lifecycle_request(method, path, "my-index"));
}
