#![cfg(test)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::truncate::truncate_evidence_payload;

fn entry(evidence: &str, score: i64) -> GuardrailValue {
    GuardrailValue::Object(
        [
            (
                "evidence".to_owned(),
                GuardrailValue::Text(evidence.to_owned()),
            ),
            ("score".to_owned(), GuardrailValue::SignedInteger(score)),
        ]
        .into_iter()
        .collect(),
    )
}

fn listed(entries: Vec<GuardrailValue>) -> GuardrailValue {
    GuardrailValue::Array(entries)
}

#[test]
fn a_small_payload_is_returned_unchanged() {
    let evidence = listed(vec![entry("short", 1)]);
    let result = truncate_evidence_payload(&evidence, 8 * 1024);
    assert_eq!(result.evidence, evidence);
    assert!(!result.was_truncated);
    assert_eq!(
        result.encoded,
        "%5B%7B%22evidence%22%3A%22short%22%2C%22score%22%3A1%7D%5D"
    );
}

#[test]
fn a_non_list_payload_that_fits_keeps_its_encoding() {
    let evidence = GuardrailValue::Text("hi".to_owned());
    let result = truncate_evidence_payload(&evidence, 100);
    assert_eq!(result.evidence, evidence);
    assert_eq!(result.encoded, "%22hi%22");
    assert!(!result.was_truncated);
}

#[test]
fn a_non_list_payload_over_the_budget_becomes_the_truncated_marker() {
    let evidence = GuardrailValue::Object(
        [(
            "evidence".to_owned(),
            GuardrailValue::Text("hello".to_owned()),
        )]
        .into_iter()
        .collect(),
    );
    let result = truncate_evidence_payload(&evidence, 5);
    assert_eq!(
        result.evidence,
        GuardrailValue::Text("[truncated]".to_owned())
    );
    assert_eq!(result.encoded, "%22%5Btruncated%5D%22");
    assert!(result.was_truncated);
}

#[test]
fn a_long_evidence_text_is_halved_until_the_encoding_fits() {
    let evidence = listed(vec![entry(&"x".repeat(100), 1)]);
    let result = truncate_evidence_payload(&evidence, 120);
    let expected = "x".repeat(13) + "...[truncated]";
    assert_eq!(
        result.evidence,
        listed(vec![GuardrailValue::Object(
            [
                ("evidence".to_owned(), GuardrailValue::Text(expected)),
                (
                    "evidence_truncated".to_owned(),
                    GuardrailValue::Boolean(true)
                ),
                ("score".to_owned(), GuardrailValue::SignedInteger(1)),
            ]
            .into_iter()
            .collect(),
        )])
    );
    assert_eq!(
        result.encoded,
        "%5B%7B%22evidence%22%3A%22xxxxxxxxxxxxx...%5Btruncated%5D%22%2C%22evidence_truncated%22%3Atrue%2C%22score%22%3A1%7D%5D"
    );
    assert!(result.was_truncated);
}

#[test]
fn an_entry_that_cannot_shrink_enough_is_dropped() {
    let evidence = listed(vec![entry(&"x".repeat(100), 1)]);
    let result = truncate_evidence_payload(&evidence, 100);
    assert_eq!(result.evidence, listed(Vec::new()));
    assert_eq!(result.encoded, "%5B%5D");
    assert!(result.was_truncated);
}

#[test]
fn an_entry_without_evidence_text_is_dropped() {
    let entry = GuardrailValue::Object(
        [("score".to_owned(), GuardrailValue::SignedInteger(1))]
            .into_iter()
            .collect(),
    );
    let result = truncate_evidence_payload(&listed(vec![entry]), 5);
    assert_eq!(result.evidence, listed(Vec::new()));
    assert!(result.was_truncated);
}

#[test]
fn a_later_entry_that_does_not_fit_leaves_the_earlier_ones() {
    let evidence = listed(vec![entry("y", 1), entry(&"z".repeat(200), 2)]);
    let result = truncate_evidence_payload(&evidence, 60);
    assert_eq!(result.evidence, listed(vec![entry("y", 1)]));
    assert!(result.was_truncated);
}

#[rstest]
#[case::encoding_of_exactly_max_bytes(7, false)]
#[case::encoding_one_byte_over_the_budget(6, true)]
#[case::no_budget_at_all(0, true)]
fn the_budget_boundary_keeps_an_encoding_of_exactly_max_bytes(
    #[case] max_bytes: usize,
    #[case] was_truncated: bool,
) {
    let evidence = listed(vec![GuardrailValue::UnsignedInteger(1)]);
    let result = truncate_evidence_payload(&evidence, max_bytes);
    if was_truncated {
        assert_eq!(result.evidence, listed(Vec::new()));
    } else {
        assert_eq!(result.evidence, evidence);
    }
    assert_eq!(result.was_truncated, was_truncated);
}

#[test]
fn an_empty_list_never_truncates_even_with_no_budget() {
    let result = truncate_evidence_payload(&listed(Vec::new()), 0);
    assert_eq!(result.evidence, listed(Vec::new()));
    assert_eq!(result.encoded, "%5B%5D");
    assert!(!result.was_truncated);
}
