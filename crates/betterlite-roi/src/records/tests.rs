#![cfg(test)]

use rstest::rstest;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::records::{
    ROIEstimate, ROIPullCommit, ROIPullEvidence, ROIPullFile, ROIPullRecord, ROIReport,
    ROIRepositoriesResponse, ROISpendRecord, ROISyncStatus,
};

const PULL: &str = r#"{"repo":"o/r","number":1,"title":"t","url":"u","login":"l","emails":[],"profile_email":"","merged_at":"m","head_sha":"s","additions":1,"deletions":2,"changed_files":3,"commit_count":4,"incomplete_metadata":false,"estimate":{"status":"error","hours":null,"reasoning":"r"},"cache_key":null}"#;
const PULL_WITHOUT_CACHE_KEY: &str = r#"{"repo":"o/r","number":1,"title":"t","url":"u","login":"l","emails":[],"profile_email":"","merged_at":"m","head_sha":"s","additions":1,"deletions":2,"changed_files":3,"commit_count":4,"incomplete_metadata":false,"estimate":{"status":"error","hours":null,"reasoning":"r"}}"#;
const REPORT: &str = r#"{"mode":"live","start":"a","end":"b","synced_at":"c","repos":["o/r"],"estimator_model":"m","estimator_prompt":"p","effort_basis":"e","spend":[{"date":"d","user_id":"u","email":"e","spend":1.25,"requests":2}],"pulls":[],"settings_fingerprint":"f"}"#;
const REPORT_WITH_ID: &str = r#"{"mode":"live","start":"a","end":"b","synced_at":"c","repos":[],"estimator_model":"m","estimator_prompt":"p","effort_basis":"e","spend":[],"pulls":[],"settings_fingerprint":"f","warnings":[],"id":"r1"}"#;
const EVIDENCE: &str = r#"{"repo":"o/r","number":1,"title":"t","body":"b","url":"u","login":"l","emails":["a@x.io"],"profile_email":"","commit_emails":["c@x.io"],"merged_at":"m","head_sha":"s","additions":1,"deletions":2,"changed_files":3,"files":[{"filename":"f.rs","status":null,"additions":1,"deletions":null}],"commits":[{"sha":"s","message":"m","additions":1,"changed_files":2}],"commit_count":1,"incomplete_metadata":true}"#;
const EVIDENCE_WITHOUT_COMMITS: &str = r#"{"repo":"o/r","number":1,"title":"t","body":"b","url":"u","login":"l","emails":[],"profile_email":"","merged_at":"m","head_sha":"s","additions":1,"deletions":2,"changed_files":3,"files":[],"commit_count":1,"incomplete_metadata":true}"#;
const SYNC_STATUS: &str = r#"{"running":false,"phase":"idle","stage":"","done":0,"total":0,"estimated":0,"reused":0,"needs_attention":0,"error":null}"#;
const SYNC_STATUS_FILLED: &str = r#"{"running":false,"phase":"idle","stage":"","done":0,"total":0,"estimated":0,"reused":0,"needs_attention":0,"error":null,"started_at":null,"finished_at":null,"next_update":null,"elapsed_seconds":0,"remaining_seconds":null}"#;
const SYNC_STATUS_WITHOUT_ERROR: &str = r#"{"running":true,"phase":"spend","stage":"","done":0,"total":0,"estimated":0,"reused":0,"needs_attention":0}"#;
const REPOSITORIES: &str = r#"{"repositories":[{"name":"o/r","visibility":"private","archived":false}],"page":1,"has_more":true}"#;

fn round_trip<T>(input: &str) -> Option<String>
where
    T: Serialize + DeserializeOwned,
{
    let record: T = serde_json::from_str(input).ok()?;
    serde_json::to_string(&record).ok()
}

#[rstest]
#[case::estimate_keeps_null_hours(
    round_trip::<ROIEstimate>(r#"{"status":"needs_review","hours":null,"reasoning":"r"}"#),
    Some(r#"{"status":"needs_review","hours":null,"reasoning":"r"}"#)
)]
#[case::estimate_keeps_present_optional_fields(
    round_trip::<ROIEstimate>(r#"{"status":"estimated","hours":1.5,"reasoning":"r","model":"m","cached":true}"#),
    Some(r#"{"status":"estimated","hours":1.5,"reasoning":"r","model":"m","cached":true}"#)
)]
#[case::estimate_requires_hours(round_trip::<ROIEstimate>(r#"{"status":"error","reasoning":"r"}"#), None)]
#[case::estimate_status_is_closed(
    round_trip::<ROIEstimate>(r#"{"status":"done","hours":null,"reasoning":"r"}"#),
    None
)]
#[case::pull_keeps_null_cache_key(round_trip::<ROIPullRecord>(PULL), Some(PULL))]
#[case::pull_requires_cache_key(round_trip::<ROIPullRecord>(PULL_WITHOUT_CACHE_KEY), None)]
#[case::report_omits_absent_fields(round_trip::<ROIReport>(REPORT), Some(REPORT))]
#[case::report_keeps_present_optional_fields(round_trip::<ROIReport>(REPORT_WITH_ID), Some(REPORT_WITH_ID))]
#[case::spend_is_a_json_number(
    round_trip::<ROISpendRecord>(r#"{"date":"d","user_id":"u","email":"e","spend":3,"requests":2}"#),
    Some(r#"{"date":"d","user_id":"u","email":"e","spend":3.0,"requests":2}"#)
)]
#[case::commit_without_changed_files(
    round_trip::<ROIPullCommit>(r#"{"sha":"s","message":"m"}"#),
    Some(r#"{"sha":"s","message":"m"}"#)
)]
#[case::commit_with_null_changed_files(
    round_trip::<ROIPullCommit>(r#"{"sha":"s","message":"m","changed_files":null}"#),
    Some(r#"{"sha":"s","message":"m","changed_files":null}"#)
)]
#[case::file_requires_every_key(
    round_trip::<ROIPullFile>(r#"{"filename":null,"status":null,"additions":1}"#),
    None
)]
#[case::evidence_nests_files_and_commits(round_trip::<ROIPullEvidence>(EVIDENCE), Some(EVIDENCE))]
#[case::evidence_requires_commits(round_trip::<ROIPullEvidence>(EVIDENCE_WITHOUT_COMMITS), None)]
#[case::sync_status_defaults(round_trip::<ROISyncStatus>(SYNC_STATUS), Some(SYNC_STATUS_FILLED))]
#[case::sync_status_requires_error(round_trip::<ROISyncStatus>(SYNC_STATUS_WITHOUT_ERROR), None)]
#[case::repositories(round_trip::<ROIRepositoriesResponse>(REPOSITORIES), Some(REPOSITORIES))]
fn record_types(#[case] actual: Option<String>, #[case] expected: Option<&str>) {
    assert_eq!(actual.as_deref(), expected);
}
