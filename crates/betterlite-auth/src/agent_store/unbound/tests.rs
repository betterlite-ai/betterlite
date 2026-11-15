#![cfg(test)]

use crate::agent_store::records::RetiredIdentityRecord;
use crate::agent_store::tests::support::{
    FakeRecords, RecordedBindingKey, store, store_without_retired,
};
use crate::identity::FailureCode;

#[tokio::test]
async fn unbound_client_without_a_retired_store_is_allowed() {
    let fake = FakeRecords::shared();
    let result = store_without_retired(&fake)
        .unbound_client("tenant-1", "client-1")
        .await;
    assert!(result.is_ok());
    assert!(fake.state().retired_identity_reads.is_empty());
}

#[tokio::test]
async fn unbound_client_reads_the_microsoft_entra_key() {
    let fake = FakeRecords::shared();
    store(&fake)
        .unbound_client("tenant-1", "client-1")
        .await
        .expect("the retired check");
    assert_eq!(
        fake.state().retired_identity_reads,
        vec![RecordedBindingKey {
            provider: String::from("microsoft_entra"),
            tenant_id: String::from("tenant-1"),
            client_id: String::from("client-1"),
        }]
    );
}

#[tokio::test]
async fn unbound_client_retired_binding_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.retired_identity = Some(RetiredIdentityRecord {
            binding_id: String::from("binding-1"),
        });
    }
    let failure = store(&fake)
        .unbound_client("tenant-1", "client-1")
        .await
        .expect_err("a retired binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "This agent identity binding has been retired"
    );
}

#[tokio::test]
async fn unbound_client_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.retired_identity_fails = true;
    }
    let failure = store(&fake)
        .unbound_client("tenant-1", "client-1")
        .await
        .expect_err("a retired check failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(
        failure.message,
        "Retired agent identity could not be checked"
    );
}
