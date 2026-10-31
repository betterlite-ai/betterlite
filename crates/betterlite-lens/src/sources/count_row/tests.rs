#![cfg(test)]

use super::CountRow;

#[test]
fn decodes_count() {
    let decoded: CountRow = serde_json::from_str(r#"{"count": 4}"#).expect("decodes");
    assert_eq!(decoded, CountRow { count: 4 });
}

#[test]
fn rejects_unknown_fields() {
    let error = serde_json::from_str::<CountRow>(r#"{"count": 4, "extra": 1}"#);
    error.unwrap_err();
}

#[test]
fn accepts_a_quoted_numeric_count() {
    let decoded: CountRow = serde_json::from_str(r#"{"count": "4"}"#).expect("decodes");
    assert_eq!(decoded, CountRow { count: 4 });
}

#[test]
fn rejects_a_non_numeric_count() {
    let error = serde_json::from_str::<CountRow>(r#"{"count": "one"}"#);
    error.unwrap_err();
}
