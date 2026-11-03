#![cfg(test)]

use std::fs;

use rstest::rstest;

use super::files::load_category_file;

#[test]
fn file_formats_apply_yaml_defaults_and_json_mappings() {
    super::tests::file_yaml_defaults_action_and_json_deduplicates_and_maps_severity();
}

#[rstest]
#[case::low(Some("1"), "low")]
#[case::medium(Some("2"), "medium")]
#[case::high_three(Some("3"), "high")]
#[case::high_four(Some("4"), "high")]
#[case::unknown_integer(Some("7"), "medium")]
#[case::fraction(Some("2.7"), "medium")]
#[case::text(Some(r#""bad""#), "medium")]
#[case::null(Some("null"), "medium")]
#[case::missing(None, "medium")]
fn file_json_preserves_name_and_severity(#[case] severity: Option<&str>, #[case] expected: &str) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("harm_toxic_abuse.JSON");
    let field = severity
        .map(|value| format!(r#", "severity": {value}"#))
        .unwrap_or_default();
    fs::write(
        &path,
        format!(r#"[{{"match": " Word |word|second|"{field}}}]"#),
    )
    .unwrap();
    let category = load_category_file(&path).unwrap();
    assert_eq!(category.category_name, "harm_toxic_abuse");
    assert_eq!(category.keywords.len(), 2);
    assert_eq!(
        category.keywords[0].get("keyword").map(String::as_str),
        Some("word")
    );
    assert_eq!(
        category.keywords[1].get("keyword").map(String::as_str),
        Some("second")
    );
    assert_eq!(
        category.keywords[0].get("severity").map(String::as_str),
        Some(expected)
    );
}
