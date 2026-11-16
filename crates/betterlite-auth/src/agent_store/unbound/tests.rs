#![cfg(test)]

use crate::agent_store::records::RetiredIdentityRecord;
use crate::agent_store::tests::support::{FakeCall, FakeRecords, RecordedBindingKey, fake_store};
use crate::agent_store::unbound;
use crate::identity::FailureCode;

#[tokio::test]
async fn unbound_client_without_a_retired_store_is_allowed() {
    let fake = FakeRecords::shared();
    let mut store = fake_store(&fake, None);
    store.retired = None;
    unbound::unbound_client(&store, "tenant-1", "client-1")
        .await
        .expect("the retired check");
    assert!(fake.state().retired_identity_reads.is_empty());
}

#[tokio::test]
async fn unbound_client_reads_the_microsoft_entra_key() {
    let fake = FakeRecords::shared();
    let store = fake_store(&fake, None);
    unbound::unbound_client(&store, "tenant-1", "client-1")
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
    fake.state().retired_identity = Some(RetiredIdentityRecord {
        binding_id: String::from("binding-1"),
    });
    let store = fake_store(&fake, None);
    let failure = unbound::unbound_client(&store, "tenant-1", "client-1")
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
    fake.state().failure = Some(FakeCall::RetiredIdentity);
    let store = fake_store(&fake, None);
    let failure = unbound::unbound_client(&store, "tenant-1", "client-1")
        .await
        .expect_err("a retired check failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(
        failure.message,
        "Retired agent identity could not be checked"
    );
}
