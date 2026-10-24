#![cfg(test)]

use rstest::rstest;

use crate::error::PathError;
use crate::paths::lookup::{find, find_or};
use crate::paths::{Document, PathLimits};

const LIMITS: PathLimits = PathLimits {
    max_path_bytes: 64,
    max_depth: 4,
    max_selectors: 2,
    max_visits: 16,
};

fn document(json: &str) -> Document {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::nested_field(r#"{"a": {"b": {"c": "value"}}}"#, "a.b.c", "null", Ok(r#""value""#))]
#[case::missing_returns_default(r#"{"a": {"b": {}}}"#, "a.b.d", r#""default""#, Ok(r#""default""#))]
#[case::escaped_dot(
    r#"{"kubernetes.io": {"namespace": "default"}}"#,
    r"kubernetes\.io.namespace",
    "null",
    Ok(r#""default""#)
)]
#[case::escaped_backslash_then_dot(r#"{"a\\.b": 1}"#, r"a\\.b", "null", Ok("1"))]
#[case::lone_backslash_is_literal(r#"{"a\\b": 1}"#, r"a\b", "null", Ok("1"))]
#[case::nul_reads_as_dot(r#"{"a.b": 1}"#, "a\0b", "null", Ok("1"))]
#[case::metadata_prefix(
    r#"{"user": {"role": "admin"}}"#,
    "metadata.user.role",
    "null",
    Ok(r#""admin""#)
)]
#[case::metadata_prefix_once(r#"{"metadata": {"x": 1}}"#, "metadata.metadata.x", "null", Ok("1"))]
#[case::bare_metadata_key(r#"{"metadata": 1}"#, "metadata", "null", Ok("1"))]
#[case::empty_path(r#"{"": 1}"#, "", r#""fallback""#, Ok(r#""fallback""#))]
#[case::empty_after_prefix(r#"{"": 2}"#, "metadata.", "null", Ok("2"))]
#[case::list_is_not_indexed(r#"{"items": ["a"]}"#, "items.0", r#""fallback""#, Ok(r#""fallback""#))]
#[case::scalar_is_not_traversed(r#"{"a": "text"}"#, "a.b", "null", Ok("null"))]
#[case::found_null_without_default(r#"{"a": null}"#, "a", "null", Ok("null"))]
#[case::found_null_typed_default(r#"{"a": null}"#, "a", r#""x""#, Ok(r#""x""#))]
#[case::string_is_not_int(r#"{"a": "text"}"#, "a", "0", Ok("0"))]
#[case::bool_counts_as_int(r#"{"a": true}"#, "a", "0", Ok("true"))]
#[case::int_is_not_bool(r#"{"a": 1}"#, "a", "false", Ok("false"))]
#[case::int_is_not_float(r#"{"a": 1}"#, "a", "1.5", Ok("1.5"))]
#[case::float_matches_float(r#"{"a": 2.5}"#, "a", "1.5", Ok("2.5"))]
#[case::list_matches_list(r#"{"a": ["x"]}"#, "a", "[]", Ok(r#"["x"]"#))]
#[case::list_is_not_dict(r#"{"a": ["x"]}"#, "a", "{}", Ok("{}"))]
#[case::dict_matches_dict(r#"{"a": {"b": 1}}"#, "a", "{}", Ok(r#"{"b": 1}"#))]
#[case::escaped_dots_are_one_part("{}", r"a\.b\.c\.d\.e", "null", Ok("null"))]
#[case::too_deep("{}", "a.b.c.d.e", "null", Err(PathError::TooDeep { limit: 4 }))]
#[case::too_long("{}", &"a".repeat(65), "null", Err(PathError::TooLong { bytes: 65, limit: 64 }))]
fn dot_paths_behavior_tests_behavior(
    #[case] data: &str,
    #[case] key_path: &str,
    #[case] default: &str,
    #[case] expected: Result<&str, PathError>,
) {
    let result = find_or(&LIMITS, &document(data), key_path, document(default));
    assert_eq!(result, expected.map(document));
}

#[rstest]
#[case::found_null(r#"{"a": null}"#, "a", Some("null"))]
#[case::missing(r#"{"a": null}"#, "b", None)]
#[case::empty_path(r#"{"": 1}"#, "", None)]
fn lookup_tells_missing_from_null(
    #[case] data: &str,
    #[case] key_path: &str,
    #[case] expected: Option<&str>,
) {
    let found = find(&LIMITS, &document(data), key_path).unwrap();
    assert_eq!(found, expected.map(document));
}
