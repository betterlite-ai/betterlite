use rstest::rstest;

use crate::api::file_credentials::hints::{ModelHints, extract_model_from_sources};
use crate::api::file_routing_ids::RoutingIdType;
use crate::api::file_routing_ids::routing::encode_file_id_with_model;

use super::support::hints;

#[rstest]
#[case::body(
    Some("model_body"),
    Some("model_query"),
    Some("model_header"),
    Some("model_body")
)]
#[case::empty_body(
    Some(""),
    Some("model_query"),
    Some("model_header"),
    Some("model_query")
)]
#[case::empty_query(None, Some(""), Some("model_header"), Some("model_header"))]
#[case::empty_header(None, None, Some(""), Some(""))]
#[case::header_only(None, None, Some("model_header"), Some("model_header"))]
#[case::none(None, None, None, None)]
fn model_hint_precedence(
    #[case] body: Option<&str>,
    #[case] query: Option<&str>,
    #[case] header: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let (embedded, from_param) =
        extract_model_from_sources("file-abc", &hints(body, query, header));
    assert_eq!(embedded, None);
    assert_eq!(from_param.as_deref(), expected);
}

#[test]
fn model_hint_precedence_empty_query_falls_to_the_header() {
    let (embedded, from_param) =
        extract_model_from_sources("file-abc", &hints(None, Some(""), Some("model_header")));
    assert_eq!(embedded, None);
    assert_eq!(from_param.as_deref(), Some("model_header"));
}

#[test]
fn model_hint_precedence_embedded_model_comes_from_the_id() {
    let encoded = encode_file_id_with_model("file-raw", "model_id", RoutingIdType::File);
    let (embedded, from_param) =
        extract_model_from_sources(&encoded, &hints(Some("model_body"), None, None));
    assert_eq!(embedded.as_deref(), Some("model_id"));
    assert_eq!(from_param.as_deref(), Some("model_body"));
}

#[test]
fn model_hints_default_carries_no_source() {
    assert_eq!(ModelHints::default(), hints(None, None, None));
    let (embedded, from_param) = extract_model_from_sources("file-abc", &ModelHints::default());
    assert_eq!((embedded, from_param), (None, None));
}
