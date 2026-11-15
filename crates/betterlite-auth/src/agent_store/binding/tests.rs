#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::agent_store::records::RetiredIdentityRecord;
use crate::agent_store::tests::support::{
    FakeCache, FakeCall, FakeRecords, RecordedBindingKey, RecordedCacheWrite, binding, store,
    store_with_cache,
};
use crate::identity::FailureCode;

const CACHE_KEY: &str = r#"agent_identity:["tenant-1", "client-1"]"#;

#[tokio::test]
async fn bound_agent_id_reads_the_microsoft_entra_key() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
    }
    let bound = store(&fake)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the identity lookup");
    assert_eq!(bound, Some(String::from("agent-1")));
    assert_eq!(
        fake.state().identity_reads,
        vec![RecordedBindingKey {
            provider: String::from("microsoft_entra"),
            tenant_id: String::from("tenant-1"),
            client_id: String::from("client-1"),
        }]
    );
}

#[tokio::test]
async fn bound_agent_id_writes_the_cache_with_the_management_ttl() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
    }
    let cache = FakeCache::shared();
    cache.state().ttl = 42.5;
    let bound = store_with_cache(&fake, &cache)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the identity lookup");
    assert_eq!(bound, Some(String::from("agent-1")));
    assert_eq!(
        cache.state().writes,
        vec![RecordedCacheWrite {
            key: String::from(CACHE_KEY),
            value: Json::Text(String::from("agent-1")),
            ttl: 42.5,
        }]
    );
}

#[tokio::test]
async fn bound_agent_id_cache_hit_skips_the_identity_read() {
    let fake = FakeRecords::shared();
    let cache = FakeCache::shared();
    cache.state().value = Some(Json::Text(String::from("cached-agent")));
    let bound = store_with_cache(&fake, &cache)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the cached binding");
    assert_eq!(bound, Some(String::from("cached-agent")));
    assert!(fake.state().identity_reads.is_empty());
    assert!(cache.state().writes.is_empty());
}

#[rstest]
#[case::null("null")]
#[case::bool("true")]
#[case::number("1")]
#[case::list("[]")]
#[case::object("{}")]
#[tokio::test]
async fn bound_agent_id_non_string_cache_value_is_a_miss(#[case] cached: &str) {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.identity = Some(binding("agent-1"));
    }
    let cache = FakeCache::shared();
    cache.state().value = Some(serde_json::from_str(cached).expect("a json cache value"));
    let bound = store_with_cache(&fake, &cache)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the identity lookup");
    assert_eq!(bound, Some(String::from("agent-1")));
    assert_eq!(fake.state().identity_reads.len(), 1);
    assert_eq!(cache.state().writes.len(), 1);
}

#[tokio::test]
async fn bound_agent_id_missing_binding_checks_the_retired_key() {
    let fake = FakeRecords::shared();
    let bound = store(&fake)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the identity lookup");
    assert!(bound.is_none());
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
async fn bound_agent_id_retired_binding_is_denied() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.retired_identity = Some(RetiredIdentityRecord {
            binding_id: String::from("binding-1"),
        });
    }
    let failure = store(&fake)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect_err("a retired binding");
    assert_eq!(failure.code, FailureCode::IdentityDenied);
    assert_eq!(
        failure.message,
        "This agent identity binding has been retired"
    );
}

#[tokio::test]
async fn bound_agent_id_read_failure_is_policy_unavailable() {
    let fake = FakeRecords::shared();
    {
        let mut state = fake.state();
        state.failure = Some(FakeCall::Identity);
    }
    let failure = store(&fake)
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect_err("an identity read failure");
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, "Agent identity could not be loaded");
}
