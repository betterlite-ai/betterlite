#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;

use super::{ParseExecutionError, parse_execution};

fn encode(json: &str) -> String {
    URL_SAFE.encode(json)
}

#[test]
fn parses_four_element_tuple() {
    let value = encode(r#"["traces", "team-a", "trace-1", "ref-1"]"#);
    let parsed = parse_execution(&value).expect("decodes");
    assert_eq!(
        parsed,
        (
            "traces".to_owned(),
            "team-a".to_owned(),
            "trace-1".to_owned(),
            "ref-1".to_owned()
        )
    );
}

#[test]
fn parses_legacy_three_element_tuple_with_empty_trace_ref() {
    let value = encode(r#"["traces", "team-a", "trace-1"]"#);
    let parsed = parse_execution(&value).expect("decodes");
    assert_eq!(
        parsed,
        (
            "traces".to_owned(),
            "team-a".to_owned(),
            "trace-1".to_owned(),
            String::new()
        )
    );
}

#[test]
fn rejects_malformed_base64() {
    let error = parse_execution("not-valid-base64!!!");
    assert!(matches!(error, Err(ParseExecutionError::Base64(_))));
}

#[test]
fn rejects_truncated_json() {
    let value = encode(r#"["traces", "team-a""#);
    let error = parse_execution(&value);
    assert!(matches!(error, Err(ParseExecutionError::Json(_))));
}

#[test]
fn rejects_wrong_element_count() {
    let value = encode(r#"["traces", "team-a"]"#);
    let error = parse_execution(&value);
    assert!(matches!(error, Err(ParseExecutionError::ElementCount(2))));
}

#[test]
fn rejects_five_element_tuple() {
    let value = encode(r#"["traces", "team-a", "trace-1", "ref-1", "extra"]"#);
    let error = parse_execution(&value);
    assert!(matches!(error, Err(ParseExecutionError::ElementCount(5))));
}
