#![cfg(test)]

use rstest::rstest;

use crate::error::PathError;
use crate::paths::delete::remove;
use crate::paths::{Document, PathLimits};

const LIMITS: PathLimits = PathLimits {
    max_path_bytes: 64,
    max_depth: 4,
    max_selectors: 2,
    max_visits: 16,
};

const TOOLS: &str = r#"{"tools": [{"name": "t1", "x": 1}, {"name": "t2", "x": 2}, "text"]}"#;

fn document(json: &str) -> Document {
    serde_json::from_str(json).unwrap()
}

#[rstest]
#[case::top_field(r#"{"a": 1, "b": 2}"#, "a", Ok(r#"{"b": 2}"#))]
#[case::nested_field(r#"{"a": {"b": 1, "c": 2}}"#, "a.b", Ok(r#"{"a": {"c": 2}}"#))]
#[case::missing_parent(r#"{"a": 1}"#, "x.y", Ok(r#"{"a": 1}"#))]
#[case::wildcard_field(
    TOOLS,
    "tools[*].x",
    Ok(r#"{"tools": [{"name": "t1"}, {"name": "t2"}, "text"]}"#)
)]
#[case::wildcard_last_keeps_items(TOOLS, "tools[*]", Ok(TOOLS))]
#[case::index_field(
    TOOLS,
    "tools[1].x",
    Ok(r#"{"tools": [{"name": "t1", "x": 1}, {"name": "t2"}, "text"]}"#)
)]
#[case::index_last_keeps_element(TOOLS, "tools[0]", Ok(TOOLS))]
#[case::index_out_of_range(TOOLS, "tools[5].x", Ok(TOOLS))]
#[case::padded_index(
    TOOLS,
    "tools[ +0_1 ].x",
    Ok(r#"{"tools": [{"name": "t1", "x": 1}, {"name": "t2"}, "text"]}"#)
)]
#[case::control_separator_is_not_space(TOOLS, "tools[\u{1c}1\u{1f}].x", Ok(TOOLS))]
#[case::unicode_space_index(
    TOOLS,
    "tools[\u{3000}1\u{85}].x",
    Ok(r#"{"tools": [{"name": "t1", "x": 1}, {"name": "t2"}, "text"]}"#)
)]
#[case::negative_zero_index(
    TOOLS,
    "tools[-0].x",
    Ok(r#"{"tools": [{"name": "t1"}, {"name": "t2", "x": 2}, "text"]}"#)
)]
#[case::negative_index(TOOLS, "tools[-1].x", Ok(TOOLS))]
#[case::word_index(TOOLS, "tools[one].x", Ok(TOOLS))]
#[case::doubled_underscore(TOOLS, "tools[0__1].x", Ok(TOOLS))]
#[case::huge_index(TOOLS, "tools[99999999999999999999999].x", Ok(TOOLS))]
#[case::nested_arrays(r#"{"a": [[{"b": 1}]]}"#, "a[0][0].b", Ok(r#"{"a": [[{}]]}"#))]
#[case::wildcard_on_object(r#"{"a": {"b": {"c": 1}}}"#, "a[*].c", Ok(r#"{"a": {"b": {"c": 1}}}"#))]
#[case::field_on_list(r#"{"a": [{"b": 1}]}"#, "a.b", Ok(r#"{"a": [{"b": 1}]}"#))]
#[case::index_is_not_a_key(
    r#"{"a": {"[0]": {"b": 1}}}"#,
    "a[0].b",
    Ok(r#"{"a": {"[0]": {"b": 1}}}"#)
)]
#[case::unclosed_bracket(r#"{"a": {"0": {"b": 1}}}"#, "a[0.b", Ok(r#"{"a": {"0": {}}}"#))]
#[case::bracket_holds_dots(r#"{"a": [{"b": 1}]}"#, "a[0.b]", Ok(r#"{"a": [{"b": 1}]}"#))]
#[case::stray_close_bracket(r#"{"a]b": {"c": 1}}"#, "a]b.c", Ok(r#"{"a]b": {}}"#))]
#[case::no_escape(
    r#"{"a.b": 1, "a\\": {"b": 2}}"#,
    r"a\.b",
    Ok(r#"{"a.b": 1, "a\\": {}}"#)
)]
#[case::empty_path(r#"{"a": 1}"#, "", Ok(r#"{"a": 1}"#))]
#[case::dots_only(r#"{"a": 1}"#, "...", Ok(r#"{"a": 1}"#))]
#[case::empty_brackets(r#"{"a": [{"b": 1}]}"#, "a[].b", Ok(r#"{"a": [{"b": 1}]}"#))]
#[case::top_level_list(r#"[{"a": 1}, 2]"#, "[*].a", Ok("[{}, 2]"))]
#[case::too_many_selectors(TOOLS, "a[0][0][0].b", Err(PathError::TooManySelectors { limit: 2 }))]
#[case::too_deep(TOOLS, "a.b.c.d.e", Err(PathError::TooDeep { limit: 4 }))]
#[case::too_long(TOOLS, &"a".repeat(65), Err(PathError::TooLong { bytes: 65, limit: 64 }))]
#[case::too_much_work(&format!(r#"{{"a": [{}{{}}]}}"#, "{}, ".repeat(14)), "a[*].b", Err(PathError::TooMuchWork { limit: 16 }))]
#[case::work_at_limit(&format!(r#"{{"a": [{}{{}}]}}"#, "{}, ".repeat(13)), "a[*].b", Ok(r#"{"a": [{}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}]}"#))]
fn dot_paths_behavior_tests_behavior(
    #[case] data: &str,
    #[case] path: &str,
    #[case] expected: Result<&str, PathError>,
) {
    let original = document(data);
    let result = remove(&LIMITS, &original, path);
    assert_eq!(result, expected.map(document));
    assert_eq!(original, document(data));
}
