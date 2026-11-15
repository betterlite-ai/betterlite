#![cfg(test)]

use crate::agent_store::tests::support::{
    FakeRecords, RecordedEnrollment, human_row, microsoft_subject, store,
};
use crate::identity::FailureCode;

#[tokio::test]
async fn enroll_create_data_carries_the_subject_and_user() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("human", "sso_interactive", Some("user-1")));
    }
    store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect("the enrollment");
    assert_eq!(
        fake.state().enrollments,
        vec![RecordedEnrollment {
            issuer: String::from("issuer-1"),
            tenant_id: String::from("tenant-1"),
            oid: String::from("oid-2"),
            user_id: String::from("user-1"),
            verified_via: String::from("sso_interactive"),
        }]
    );
}

#[tokio::test]
async fn enroll_matching_row_succeeds() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("human", "sso_interactive", Some("user-1")));
    }
    let result = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn enroll_other_user_row_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("human", "sso_interactive", Some("user-9")));
    }
    let failure = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("a subject bound to another user");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Microsoft subject is already bound to another local identity"
    );
}

#[tokio::test]
async fn enroll_non_human_row_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("application", "sso_interactive", Some("user-1")));
    }
    let failure = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("a non-human subject");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Microsoft subject is already bound to another local identity"
    );
}

#[tokio::test]
async fn enroll_other_verified_via_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("human", "sso_other", Some("user-1")));
    }
    let failure = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("a subject verified another way");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Microsoft subject is already bound to another local identity"
    );
}

#[tokio::test]
async fn enroll_null_user_row_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_row = Some(human_row("human", "sso_interactive", None));
    }
    let failure = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("a subject without a user");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "Microsoft subject is already bound to another local identity"
    );
}

#[tokio::test]
async fn enroll_upsert_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.enrollment_fails = true;
    }
    let failure = store(&fake)
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("an upsert failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(
        failure.message,
        "Microsoft subject enrollment is unavailable"
    );
}
