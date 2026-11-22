#![cfg(test)]

mod corpus;
mod lines;
mod parse;
mod policy;
mod routes;
mod scan;
mod support;

use std::collections::{BTreeMap, BTreeSet};

use rstest::rstest;

use super::records::{
    BatchRejection, BatchScanFailure, BatchScanResult, RecordChange, RecordDropped, RecordRedacted,
    UnparseableRecord, UnroutableRecord, UnscannableRecord,
};
use super::{
    BODY_SHAPE_CALL_TYPES, CUSTOM_ID_LOG_LIMIT, EMPTY_MAPPING, INJECTED_KEYS, REWRITE_SPOOL_BYTES,
    ROUTE_APPLIED_KEY, SCAN_METADATA_BAGS, SCAN_METADATA_KEYS, SCAN_WINDOW, SCANNABLE_CALL_TYPES,
    SUMMARY_LIMIT,
};
use crate::routes::CallType;

#[test]
fn keeps_the_literal_constants() {
    assert_eq!(EMPTY_MAPPING, BTreeMap::new());
    assert_eq!(SCAN_WINDOW, 32);
    assert_eq!(REWRITE_SPOOL_BYTES, 0x0010_0000);
    assert_eq!(CUSTOM_ID_LOG_LIMIT, 128);
    assert_eq!(SUMMARY_LIMIT, 50);
    assert_eq!(SCAN_METADATA_BAGS, ["litellm_metadata", "metadata"]);
    assert_eq!(ROUTE_APPLIED_KEY, "sensitive_data_routing_applied");
    assert_eq!(
        BTreeSet::from(INJECTED_KEYS),
        BTreeSet::from(["guardrails", "litellm_metadata", "metadata"])
    );
}

#[test]
fn keeps_the_scan_key_and_call_type_sets() {
    assert_eq!(
        BTreeSet::from(SCAN_METADATA_KEYS),
        BTreeSet::from([
            "_guardrail_pipelines",
            "_pipeline_managed_guardrails",
            "guardrails",
            "headers",
            "tags",
            "user_api_key_metadata",
            "user_api_key_team_metadata",
        ])
    );
    assert_eq!(
        SCANNABLE_CALL_TYPES,
        [
            CallType::Acompletion,
            CallType::AtextCompletion,
            CallType::Aembedding,
            CallType::Aresponses,
            CallType::AnthropicMessages,
        ]
    );
    assert_eq!(
        BODY_SHAPE_CALL_TYPES,
        [
            ("messages", CallType::Acompletion),
            ("prompt", CallType::AtextCompletion),
            ("input", CallType::Aembedding),
        ]
    );
}

fn redacted(line_number: usize, custom_id: &str) -> RecordChange {
    RecordChange::Redacted(RecordRedacted {
        line_number,
        custom_id: Some(custom_id.to_owned()),
        offset: 4,
        length: 8,
    })
}

fn dropped(line_number: usize, custom_id: &str, guardrail: Option<&str>) -> RecordChange {
    RecordChange::Dropped(RecordDropped {
        line_number,
        custom_id: Some(custom_id.to_owned()),
        guardrail: guardrail.map(str::to_owned),
    })
}

const fn result(changes: Vec<RecordChange>, scanned_records: usize) -> BatchScanResult<()> {
    BatchScanResult {
        changes,
        scanned_records,
        redactions: (),
    }
}

#[test]
fn reports_each_change_without_its_spool_offsets() {
    let scan = result(vec![redacted(2, "a"), dropped(3, "b", Some("g"))], 3);
    assert_eq!(scan.submitted_records(), 2);
    assert_eq!(
        serde_json::to_string(&scan.report()).unwrap(),
        r#"{"submitted_records":2,"modified_records":[{"line":2,"custom_id":"a","action":"redacted","guardrail":null},{"line":3,"custom_id":"b","action":"dropped","guardrail":"g"}]}"#
    );
    assert_eq!(
        scan.summary(),
        "line 2 (custom_id a) redacted, line 3 (custom_id b) dropped"
    );
}

#[test]
fn an_untouched_scan_summarizes_as_nothing() {
    let scan = result(Vec::new(), 4);
    assert_eq!(scan.submitted_records(), 4);
    assert_eq!(scan.summary(), "");
    assert!(scan.report().modified_records.is_empty());
}

#[rstest]
#[case::at_the_limit(50, "")]
#[case::one_past(51, ", and 1 more")]
#[case::two_past(52, ", and 2 more")]
fn summary_shows_the_first_fifty(#[case] count: usize, #[case] tail: &str) {
    let changes: Vec<RecordChange> = (1..=count).map(|line| dropped(line, "x", None)).collect();
    let scan = result(changes, count);
    let summary = scan.summary();
    assert!(summary.starts_with("line 1 (custom_id x) dropped, line 2 (custom_id x) dropped"));
    assert!(summary.contains("line 50 (custom_id x) dropped"));
    assert!(!summary.contains("line 51 "));
    assert!(summary.ends_with(&format!("dropped{tail}")));
    assert_eq!(scan.submitted_records(), 0);
}

#[test]
fn a_change_without_a_custom_id_is_described_by_its_line() {
    let change = RecordChange::Redacted(RecordRedacted {
        line_number: 7,
        custom_id: None,
        offset: 0,
        length: 1,
    });
    let scan = result(vec![change], 9);
    assert_eq!(scan.summary(), "line 7 redacted");
    assert_eq!(scan.submitted_records(), 9);
}

#[test]
fn records_keep_their_fields() {
    let unscannable = UnscannableRecord {
        line_number: 2,
        custom_id: Some("a".to_owned()),
        url: Some("/v1/unknown".to_owned()),
    };
    assert_eq!(
        (
            unscannable.line_number,
            unscannable.custom_id.as_deref(),
            unscannable.url.as_deref()
        ),
        (2, Some("a"), Some("/v1/unknown"))
    );
    let RecordChange::Dropped(record) = dropped(2, "a", None) else {
        panic!("a dropped change");
    };
    assert_eq!(record.guardrail, None);
    let RecordChange::Redacted(spooled) = redacted(2, "a") else {
        panic!("a redacted change");
    };
    assert_eq!((spooled.offset, spooled.length), (4, 8));
}

fn unscannable(custom_id: Option<&str>, url: Option<&str>) -> BatchRejection {
    BatchRejection::Failure(BatchScanFailure::Unscannable(UnscannableRecord {
        line_number: 2,
        custom_id: custom_id.map(str::to_owned),
        url: url.map(str::to_owned),
    }))
}

fn unroutable(custom_id: Option<&str>, guardrail: Option<&str>) -> BatchRejection {
    BatchRejection::Failure(BatchScanFailure::Unroutable(UnroutableRecord {
        line_number: 2,
        custom_id: custom_id.map(str::to_owned),
        guardrail: guardrail.map(str::to_owned),
    }))
}

const NO_BODY: &str = " and its body has no messages, prompt or input, so guardrails cannot read it. Give the record a chat, completion, embedding, responses or messages body";
const ONE_PROVIDER: &str = ", and every record of a batch file goes to one provider, so the file cannot be submitted. Send that record outside the batch";

#[rstest]
#[case::unparseable(
    BatchRejection::Failure(BatchScanFailure::Unparseable(UnparseableRecord { line_number: 2 })),
    "The 'body' of batch input line 2 is not an object, so guardrails cannot be applied to it".to_owned()
)]
#[case::unscannable_without_url(
    unscannable(Some("a"), None),
    format!("Batch input line 2 (custom_id a) targets no url{NO_BODY}")
)]
#[case::unscannable_empty_url(
    unscannable(Some("a"), Some("")),
    format!("Batch input line 2 (custom_id a) targets no url{NO_BODY}")
)]
#[case::unscannable_names_url(
    unscannable(Some("a"), Some("/v1/unknown")),
    format!("Batch input line 2 (custom_id a) targets /v1/unknown{NO_BODY}")
)]
#[case::unroutable_names_guardrail(
    unroutable(Some("a"), Some("g")),
    format!("Batch input line 2 (custom_id a) was routed to a different model by g{ONE_PROVIDER}")
)]
#[case::unroutable_without_names(
    unroutable(None, None),
    format!("Batch input line 2 was routed to a different model by a guardrail{ONE_PROVIDER}")
)]
#[case::unroutable_empty_guardrail(
    unroutable(Some(""), Some("")),
    format!("Batch input line 2 was routed to a different model by a guardrail{ONE_PROVIDER}")
)]
#[case::nothing_to_submit(
    BatchRejection::NothingToSubmit,
    "Every record in the batch input file was blocked by a guardrail, so there is nothing left to submit".to_owned()
)]
fn rejects_with_the_public_message(#[case] rejection: BatchRejection, #[case] message: String) {
    assert_eq!(BatchRejection::STATUS, 400);
    assert_eq!(rejection.to_string(), message);
}
