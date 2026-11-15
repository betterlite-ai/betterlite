#![cfg(test)]

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use crate::agent_store::tests::support::{
    FakeCall, FakeRecords, RecordedSubjectKey, human_row, store,
};
use crate::identity::FailureCode;

#[rstest]
#[case::null("null")]
#[case::bool("true")]
#[case::number("1")]
#[case::array("[]")]
#[case::object("{}")]
#[tokio::test]
async fn subject_non_string_oid_is_none(#[case] oid: &str) {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.subject = Some(human_row("human", "sso_interactive", Some("user-1")));
    }
    let oid: JsonData = serde_json::from_str(oid).expect("a json oid");
    let found = store(&fake)
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect("the subject lookup");
    assert!(found.is_none());
    assert!(fake.state().subject_reads.is_empty());
}

#[tokio::test]
async fn subject_missing_oid_is_none() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.subject = Some(human_row("human", "sso_interactive", Some("user-1")));
    }
    let found = store(&fake)
        .subject("issuer-1", "tenant-1", None)
        .await
        .expect("the subject lookup");
    assert!(found.is_none());
    assert!(fake.state().subject_reads.is_empty());
}

#[tokio::test]
async fn subject_reads_the_verified_subject_key() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.subject = Some(human_row("human", "sso_interactive", Some("user-1")));
    }
    let oid = JsonData::String(String::from("oid-2"));
    let found = store(&fake)
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect("the subject lookup")
        .expect("the stored subject");
    assert_eq!(found.user_id.as_deref(), Some("user-1"));
    assert_eq!(
        fake.state().subject_reads,
        vec![RecordedSubjectKey {
            issuer: String::from("issuer-1"),
            tenant_id: String::from("tenant-1"),
            oid: String::from("oid-2"),
        }]
    );
}

#[tokio::test]
async fn subject_missing_returns_none() {
    let fake = FakeRecords::shared();
    let oid = JsonData::String(String::from("oid-2"));
    let found = store(&fake)
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect("the subject lookup");
    assert!(found.is_none());
    assert_eq!(fake.state().subject_reads.len(), 1);
}

#[tokio::test]
async fn subject_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.failure = Some(FakeCall::Subject);
    }
    let oid = JsonData::String(String::from("oid-2"));
    let failure = store(&fake)
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect_err("a subject read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Subject classification is unavailable");
}
