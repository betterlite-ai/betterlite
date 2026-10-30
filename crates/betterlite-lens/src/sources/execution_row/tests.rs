#![cfg(test)]

use super::ExecutionRow;
use crate::models::execution::ExecutionSource;

fn valid_json() -> &'static str {
    r#"{"source": "traces", "trace_id": "t1", "team_id": "team-a", "name": "run",
        "start_time": "2026-01-01T00:00:00Z", "span_count": 3, "root_seen": 1, "eligible": 9}"#
}

#[test]
fn decodes_with_defaults() {
    let decoded: ExecutionRow = serde_json::from_str(valid_json()).expect("decodes");
    assert_eq!(decoded.selection_key, "");
    assert_eq!(decoded.trace_ref, "");
    assert_eq!(decoded.selected, 0);
    assert_eq!(decoded.service, "");
    assert!(decoded.attributes.is_empty());
    assert_eq!(decoded.source, ExecutionSource::Traces);
}

#[test]
fn decodes_all_fields() {
    let decoded: ExecutionRow = serde_json::from_str(
        r#"{"selection_key": "k", "source": "requests", "trace_id": "t1", "trace_ref": "r1",
            "team_id": "team-a", "name": "run", "start_time": "2026-01-01T00:00:00Z",
            "span_count": 3, "root_seen": 0, "eligible": 9, "selected": 2, "service": "svc",
            "attributes": [["k1", "v1"]]}"#,
    )
    .expect("decodes");
    assert_eq!(decoded.selection_key, "k");
    assert_eq!(decoded.trace_ref, "r1");
    assert_eq!(decoded.selected, 2);
    assert_eq!(decoded.service, "svc");
    assert_eq!(decoded.attributes, vec![("k1".to_owned(), "v1".to_owned())]);
}

#[test]
fn rejects_unknown_fields() {
    let with_extra = valid_json().replacen('}', r#", "extra": 1}"#, 1);
    let error = serde_json::from_str::<ExecutionRow>(&with_extra);
    error.unwrap_err();
}

#[test]
fn rejects_invalid_source() {
    let error = serde_json::from_str::<ExecutionRow>(&valid_json().replace("traces", "both"));
    error.unwrap_err();
}

#[test]
fn accepts_quoted_counters_from_clickhouse() {
    let decoded: ExecutionRow = serde_json::from_str(
        r#"{"source": "traces", "trace_id": "t1", "team_id": "team-a", "name": "run",
            "start_time": "2026-01-01T00:00:00Z", "span_count": "3", "root_seen": "1",
            "eligible": "9", "selected": "2"}"#,
    )
    .expect("decodes");
    assert_eq!(decoded.span_count, 3);
    assert_eq!(decoded.root_seen, 1);
    assert_eq!(decoded.eligible, 9);
    assert_eq!(decoded.selected, 2);
}
