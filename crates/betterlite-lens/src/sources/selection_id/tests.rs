#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;

use super::selection_id;
use crate::sources::parse_execution::ParseExecutionError;

fn encode(json: &str) -> String {
    URL_SAFE.encode(json)
}

#[test]
fn uses_trace_ref_when_present() {
    let value = encode(r#"["traces", "team-a", "trace-1", "ref-1"]"#);
    let id = selection_id(&value).expect("decodes");
    assert_eq!(id, "traces\0team-a\0ref-1");
}

#[test]
fn falls_back_to_trace_id_when_trace_ref_is_empty() {
    let value = encode(r#"["traces", "team-a", "trace-1", ""]"#);
    let id = selection_id(&value).expect("decodes");
    assert_eq!(id, "traces\0team-a\0trace-1");
}

#[test]
fn falls_back_to_trace_id_for_legacy_three_element_form() {
    let value = encode(r#"["requests", "team-a", "trace-1"]"#);
    let id = selection_id(&value).expect("decodes");
    assert_eq!(id, "requests\0team-a\0trace-1");
}

#[test]
fn propagates_parse_errors() {
    let error = selection_id("not-valid-base64!!!");
    assert!(matches!(error, Err(ParseExecutionError::Base64(_))));
}
