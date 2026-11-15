#![cfg(test)]

pub(crate) mod support;

use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;

use self::support::{FakeCache, application_claims, authentication_context, microsoft_subject};
use crate::agent_store::AgentIdentityStore;
use crate::identity::FailureCode;

#[test]
fn init_from_client_builds_every_repository() {
    let client = PostgresStore { pool: None };
    let store = AgentIdentityStore::from_client(&client, None);
    assert!(store.retired.is_some());
    assert!(store.retired_agents.is_some());
    assert!(store.cache.is_none());
}

#[tokio::test]
async fn init_from_client_keeps_the_management_cache() {
    let cache = FakeCache::shared();
    {
        let mut state = cache.state();
        state.value = Some(Json::Text(String::from("cached-agent")));
    }
    let client = PostgresStore { pool: None };
    let store = AgentIdentityStore::from_client(&client, Some(Arc::clone(&cache)));
    let bound = store
        .bound_agent_id("tenant-1", "client-1")
        .await
        .expect("the cached binding");
    assert_eq!(bound, Some(String::from("cached-agent")));
}

#[tokio::test]
async fn init_without_a_database_reports_each_repository_unavailable() {
    let store = AgentIdentityStore::from_client(&PostgresStore { pool: None }, None);
    let agent = store.agent("agent-1").await.expect_err("no database");
    assert_eq!(agent.code, FailureCode::PolicyUnavailable);
    assert_eq!(agent.message, "Agent policy could not be loaded");
    let identity = store
        .resolve_verified_claims(&application_claims())
        .await
        .expect_err("no database");
    assert_eq!(identity.message, "Agent identity could not be loaded");
    let retired = store
        .unbound_client("tenant-1", "client-1")
        .await
        .expect_err("no database");
    assert_eq!(
        retired.message,
        "Retired agent identity could not be checked"
    );
    let history = store
        .retired_agent("agent-1")
        .await
        .expect_err("no database");
    assert_eq!(history.message, "Agent history is unavailable");
    let oid = JsonData::String(String::from("oid-2"));
    let subject = store
        .subject("issuer-1", "tenant-1", Some(&oid))
        .await
        .expect_err("no database");
    assert_eq!(subject.message, "Subject classification is unavailable");
    let authentication = store
        .record_authentication(&authentication_context("agent-1", Some("rev-1")))
        .await
        .expect_err("no database");
    assert_eq!(
        authentication.message,
        "Agent authentication could not be recorded"
    );
    let enrollment = store
        .enroll_interactive_human(&microsoft_subject(), "user-1")
        .await
        .expect_err("no database");
    assert_eq!(
        enrollment.message,
        "Microsoft subject enrollment is unavailable"
    );
}
