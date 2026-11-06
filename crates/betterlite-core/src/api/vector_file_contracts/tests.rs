#![cfg(test)]

use betterlite_openai::api::vector_file_contracts::records::FileParams;
use rstest::rstest;

use super::{FileParamsKind, filter_file_params};

fn object(text: &str) -> FileParams {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::create_keeps_three_fields(
    FileParamsKind::Create,
    r#"{"file_id": "file_a", "attributes": {}, "chunking_strategy": {"type": "auto"}, "unknown": 1}"#,
    r#"{"file_id": "file_a", "attributes": {}, "chunking_strategy": {"type": "auto"}}"#
)]
#[case::create_drops_nulls(
    FileParamsKind::Create,
    r#"{"file_id": "file_a", "attributes": null, "chunking_strategy": null}"#,
    r#"{"file_id": "file_a"}"#
)]
#[case::create_does_not_synthesize_file_id(
    FileParamsKind::Create,
    r#"{"attributes": {"k": "v"}}"#,
    r#"{"attributes": {"k": "v"}}"#
)]
#[case::create_static_chunking_survives(
    FileParamsKind::Create,
    r#"{"chunking_strategy": {"type": "static", "static": {"max_chunk_size_tokens": 800, "chunk_overlap_tokens": 400}}}"#,
    r#"{"chunking_strategy": {"type": "static", "static": {"max_chunk_size_tokens": 800, "chunk_overlap_tokens": 400}}}"#
)]
#[case::update_keeps_empty_attributes(
    FileParamsKind::Update,
    r#"{"attributes": {}, "file_id": "file_a", "unknown": 1}"#,
    r#"{"attributes": {}}"#
)]
#[case::update_null_attributes_disappear(FileParamsKind::Update, r#"{"attributes": null}"#, "{}")]
#[case::list_keeps_five_fields(
    FileParamsKind::List,
    r#"{"after": "", "before": "file_b", "filter": "failed", "limit": 0, "order": "asc", "unknown": 1}"#,
    r#"{"after": "", "before": "file_b", "filter": "failed", "limit": 0, "order": "asc"}"#
)]
#[case::list_drops_nulls(
    FileParamsKind::List,
    r#"{"after": null, "before": null, "filter": null, "limit": null, "order": null}"#,
    "{}"
)]
#[case::list_does_not_validate_values(
    FileParamsKind::List,
    r#"{"filter": "unexpected", "order": "sideways", "limit": -1}"#,
    r#"{"filter": "unexpected", "order": "sideways", "limit": -1}"#
)]
#[case::falsy_values_survive(
    FileParamsKind::List,
    r#"{"limit": 0, "after": "", "filter": false}"#,
    r#"{"limit": 0, "after": "", "filter": false}"#
)]
#[case::empty_collections_survive(
    FileParamsKind::Create,
    r#"{"attributes": [], "chunking_strategy": {}}"#,
    r#"{"attributes": [], "chunking_strategy": {}}"#
)]
fn filters_keep_known_non_null_values(
    #[case] kind: FileParamsKind,
    #[case] params: &str,
    #[case] expected: &str,
) {
    assert_eq!(filter_file_params(kind, &object(params)), object(expected));
}

#[test]
fn the_input_params_are_left_untouched() {
    let params = object(r#"{"attributes": null, "unknown": 1}"#);
    let before = params.clone();
    let _filtered = filter_file_params(FileParamsKind::Update, &params);
    assert_eq!(params, before);
}
