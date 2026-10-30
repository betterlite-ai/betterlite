#![cfg(test)]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;

use super::execution_id;

#[test]
fn encodes_base64_urlsafe_padded_json_array() {
    let id = execution_id("traces", "team-a", "trace-1", "ref-1");
    let decoded = URL_SAFE.decode(&id).expect("valid base64");
    assert_eq!(
        decoded,
        br#"["traces", "team-a", "trace-1", "ref-1"]"#.to_vec()
    );
}

#[test]
fn encodes_empty_trace_ref() {
    let id = execution_id("requests", "team-a", "trace-1", "");
    let decoded = URL_SAFE.decode(&id).expect("valid base64");
    assert_eq!(
        decoded,
        br#"["requests", "team-a", "trace-1", ""]"#.to_vec()
    );
}

#[test]
fn escapes_non_ascii_as_unicode_escapes() {
    let id = execution_id("traces", "team-a", "trace-1", "ref>>>\u{e9}");
    let decoded = URL_SAFE.decode(&id).expect("valid base64");
    let expected = "[\"traces\", \"team-a\", \"trace-1\", \"ref>>>\\u00e9\"]";
    assert_eq!(decoded, expected.as_bytes().to_vec());
}

#[test]
fn output_contains_only_urlsafe_characters() {
    let id = execution_id("traces", "team/a+b", "trace_1", "ref=1");
    assert!(id.chars().all(|value| value.is_ascii_alphanumeric()
        || value == '-'
        || value == '_'
        || value == '='));
}
