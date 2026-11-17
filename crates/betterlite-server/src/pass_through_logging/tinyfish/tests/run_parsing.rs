#![cfg(test)]

use rstest::rstest;

use super::records::parse_run;
use super::support::{capture_logs, json};

#[test]
fn parses_a_completed_run_with_every_field() {
    let parsed = parse_run(&json(
        r#"{"run_id": "run-1", "status": "COMPLETED", "num_of_steps": 3, "result": {"ok": true}, "error": null, "type": "automation"}"#,
    ))
    .expect("the run parses");
    assert_eq!(parsed.run_id, Some(Some("run-1".to_owned())));
    assert_eq!(parsed.status, Some(Some("COMPLETED".to_owned())));
    assert_eq!(parsed.num_of_steps, Some(Some(3)));
    assert_eq!(parsed.result, Some(Some(json(r#"{"ok": true}"#))));
    assert_eq!(parsed.error, Some(None));
    assert_eq!(parsed.run_type, Some(Some("automation".to_owned())));
}

#[test]
fn leaves_absent_fields_absent() {
    let parsed = parse_run(&json("{}")).expect("an empty run parses");
    assert_eq!(parsed.run_id, None);
    assert_eq!(parsed.status, None);
    assert_eq!(parsed.num_of_steps, None);
    assert_eq!(parsed.result, None);
    assert_eq!(parsed.error, None);
    assert_eq!(parsed.run_type, None);
    assert_eq!(
        serde_json::to_string(&parsed).expect("the run encodes"),
        "{}"
    );
}

#[test]
fn keeps_an_explicit_null_as_null() {
    let parsed = parse_run(&json(
        r#"{"run_id": null, "status": null, "num_of_steps": null, "result": null, "error": null, "type": null}"#,
    ))
    .expect("a null-heavy run parses");
    assert_eq!(parsed.run_id, Some(None));
    assert_eq!(parsed.status, Some(None));
    assert_eq!(parsed.num_of_steps, Some(None));
    assert_eq!(parsed.result, Some(None));
    assert_eq!(parsed.error, Some(None));
    assert_eq!(parsed.run_type, Some(None));
    assert_eq!(
        serde_json::to_string(&parsed).expect("the run encodes"),
        r#"{"run_id":null,"status":null,"num_of_steps":null,"result":null,"error":null,"type":null}"#
    );
}

#[test]
fn drops_keys_a_run_does_not_declare() {
    let parsed = parse_run(&json(
        r#"{"run_id": "run-1", "unknown": {"nested": [1, 2]}}"#,
    ))
    .expect("an unknown key is dropped");
    assert_eq!(
        serde_json::to_string(&parsed).expect("the run encodes"),
        r#"{"run_id":"run-1"}"#
    );
}

#[rstest]
#[case::string_steps(r#"{"num_of_steps": "3"}"#)]
#[case::fractional_steps(r#"{"num_of_steps": 3.5}"#)]
#[case::boolean_steps(r#"{"num_of_steps": true}"#)]
#[case::number_run_id(r#"{"run_id": 5}"#)]
#[case::number_status(r#"{"status": 5}"#)]
#[case::list_payload("[1, 2]")]
#[case::string_payload(r#""text""#)]
#[case::null_payload("null")]
fn rejects_a_payload_the_record_cannot_hold(#[case] text: &str) {
    let (logs, _guard) = capture_logs();
    assert_eq!(parse_run(&json(text)), None);
    assert!(logs.contents().contains("unexpected run object shape"));
}
