use super::super::super::diagnostics::v2_payload_missing_fields;
use super::Record;

fn payload(text: &str) -> Record {
    serde_json::from_str(text).unwrap()
}

#[test]
fn a_complete_payload_reports_nothing() {
    let complete = payload(
        r#"{"traces": [{"id": "t", "type": "trace", "spans": [{"id": "s", "trace_id": "t", "parent_id": "t"}]}]}"#,
    );
    assert!(v2_payload_missing_fields(&complete).is_empty());
}

#[test]
fn an_empty_trace_list_is_reported() {
    assert_eq!(
        v2_payload_missing_fields(&payload(r#"{"traces": []}"#)),
        ["traces"]
    );
    assert_eq!(v2_payload_missing_fields(&payload("{}")), ["traces"]);
}

#[test]
fn missing_trace_and_span_fields_are_named_by_position() {
    let broken = payload(
        r#"{"traces": [{"id": "t", "spans": [{"id": "s", "trace_id": "other"}]}, "skipped"]}"#,
    );
    assert_eq!(
        v2_payload_missing_fields(&broken),
        [
            "traces[0].type",
            "traces[0].spans[0].parent_id",
            "traces[0].spans[0].trace_id mismatch",
        ]
    );
}

#[test]
fn a_span_without_a_trace_id_is_reported_once_when_the_trace_has_one() {
    let broken = payload(
        r#"{"traces": [{"id": "t", "type": "trace", "spans": [{"id": "s", "parent_id": "t"}]}]}"#,
    );
    assert_eq!(
        v2_payload_missing_fields(&broken),
        [
            "traces[0].spans[0].trace_id",
            "traces[0].spans[0].trace_id mismatch",
        ]
    );
}
