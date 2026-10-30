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
fn rejects_non_integer_count() {
    let error = serde_json::from_str::<CountRow>(r#"{"count": "4"}"#);
    error.unwrap_err();
}
