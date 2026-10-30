#![cfg(test)]

use super::PartRow;

fn valid_json() -> &'static str {
    r#"{"span_id": "s1", "parent_span_id": "", "name": "root", "kind": "agent", "content": "hi", "truncated": 0}"#
}

#[test]
fn decodes_all_fields() {
    let decoded: PartRow = serde_json::from_str(valid_json()).expect("decodes");
    assert_eq!(
        decoded,
        PartRow {
            span_id: "s1".to_owned(),
            parent_span_id: String::new(),
            name: "root".to_owned(),
            kind: "agent".to_owned(),
            content: "hi".to_owned(),
            truncated: 0,
        }
    );
}

#[test]
fn rejects_unknown_fields() {
    let error = serde_json::from_str::<PartRow>(
        r#"{"span_id": "s1", "parent_span_id": "", "name": "root", "kind": "agent", "content": "hi", "truncated": 0, "extra": 1}"#,
    );
    error.unwrap_err();
}

#[test]
fn rejects_missing_name() {
    let error = serde_json::from_str::<PartRow>(
        r#"{"span_id": "s1", "parent_span_id": "", "kind": "agent", "content": "hi", "truncated": 0}"#,
    );
    error.unwrap_err();
}

#[test]
fn rejects_missing_parent_span_id() {
    let error = serde_json::from_str::<PartRow>(
        r#"{"span_id": "s1", "name": "root", "kind": "agent", "content": "hi", "truncated": 0}"#,
    );
    error.unwrap_err();
}

#[test]
fn rejects_non_integer_truncated() {
    let error = serde_json::from_str::<PartRow>(
        r#"{"span_id": "s1", "parent_span_id": "", "name": "root", "kind": "agent", "content": "hi", "truncated": false}"#,
    );
    error.unwrap_err();
}
