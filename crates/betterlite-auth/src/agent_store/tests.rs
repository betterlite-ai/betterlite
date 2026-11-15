#![cfg(test)]

pub(crate) mod support;

use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;

use self::support::{FakeCache, application_claims, authentication_context, microsoft_subject};
use crate::agent_store::AgentIdentityStore;
use crate::agent_store::{
    agent, authentication, binding, enrollment, resolve, retired, subject, unbound,
};
use crate::identity::{AgentIdentityFailure, FailureCode};

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
    let store = AgentIdentityStore::from_client(&client, Some(Arc::<FakeCache>::clone(&cache)));
    let bound = binding::bound_agent_id(&store, "tenant-1", "client-1")
        .await
        .expect("the cached binding");
    assert_eq!(bound, Some(String::from("cached-agent")));
}

fn assert_unavailable(failure: &AgentIdentityFailure, message: &str) {
    assert_eq!(failure.code, FailureCode::PolicyUnavailable);
    assert_eq!(failure.message, message);
}

#[tokio::test]
async fn init_without_a_database_reports_the_binding_repositories_unavailable() {
    let store = AgentIdentityStore::from_client(&PostgresStore { pool: None }, None);
    let agent_failure = agent::load(&store, "agent-1")
        .await
        .expect_err("no database");
    assert_unavailable(&agent_failure, "Agent policy could not be loaded");
    let identity_failure = resolve::resolve_verified_claims(&store, &application_claims())
        .await
        .expect_err("no database");
    assert_unavailable(&identity_failure, "Agent identity could not be loaded");
    let unbound_failure = unbound::unbound_client(&store, "tenant-1", "client-1")
        .await
        .expect_err("no database");
    assert_unavailable(
        &unbound_failure,
        "Retired agent identity could not be checked",
    );
    let history_failure = retired::agent_is_retired(&store, "agent-1")
        .await
        .expect_err("no database");
    assert_unavailable(&history_failure, "Agent history is unavailable");
}

#[tokio::test]
async fn init_without_a_database_reports_the_human_repositories_unavailable() {
    let store = AgentIdentityStore::from_client(&PostgresStore { pool: None }, None);
    let oid = JsonData::String(String::from("oid-2"));
    let subject_failure = subject::find(&store, "issuer-1", "tenant-1", Some(&oid))
        .await
        .expect_err("no database");
    assert_unavailable(&subject_failure, "Subject classification is unavailable");
    let context = authentication_context("agent-1", Some("rev-1"));
    let authentication_failure = authentication::record_authentication(&store, &context)
        .await
        .expect_err("no database");
    assert_unavailable(
        &authentication_failure,
        "Agent authentication could not be recorded",
    );
    let enrollment_failure =
        enrollment::enroll_interactive_human(&store, &microsoft_subject(), "user-1")
            .await
            .expect_err("no database");
    assert_unavailable(
        &enrollment_failure,
        "Microsoft subject enrollment is unavailable",
    );
}
