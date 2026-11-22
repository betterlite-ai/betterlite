#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use rstest::rstest;

use crate::agent_access::resolver::AgentAccessResolver;
use crate::agent_policy::ManagedAgentMode;
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::admit_managed_actor;
use crate::managed_admission::tests::{
    FakeGrants, FakeHumans, FakeStore, FakeTargets, MANAGED, PLAIN, agent, context, ids, key,
    refusal, registry, resolver,
};

const BOUND: &str = r#"{"agent_id":"bound","agent_name":"bound","agent_card_params":{},"identity":{"agent_id":"bound","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;
const DISABLED: &str = r#"{"agent_id":"agent-1","agent_name":"managed-agent","agent_card_params":{},"enabled":false,"identity_managed":true,"identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;
const AUTONOMOUS: &str = r#"{"agent_id":"agent-1","agent_name":"managed-agent","agent_card_params":{},"identity_managed":true,"execution_mode":"autonomous","identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;

fn claims(auth: &mut UserAPIKeyAuth) {
    auth.extra.jwt_claims = Some(Payload(BTreeMap::from([(
        String::from("sub"),
        JsonData::String(String::from("human")),
    )])));
}

#[tokio::test]
async fn the_delegation_flag_is_cleared_without_an_agent_id() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"user_id":"human"}"#);
    auth.managed_delegation_verified = true;
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
    assert!(!auth.take_delegation_verified());
}

#[tokio::test]
async fn a_key_without_an_agent_id_is_admitted_untouched() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
    assert!(auth.extra.managed_agent_policy.is_none());
    assert!(!auth.extra.requires_fresh_policy);
}

#[tokio::test]
async fn a_plain_registered_agent_without_a_database_is_admitted() {
    let registry = registry(&[PLAIN]);
    let mut auth = key(r#"{"agent_id":"plain"}"#);
    let access = resolver(&registry, &[], None);
    let store: Option<&FakeStore> = None;
    admit_managed_actor(&mut auth, store, &registry, &access)
        .await
        .unwrap();
    assert!(auth.extra.managed_agent_policy.is_none());
}

#[tokio::test]
async fn a_context_without_a_database_is_policy_unavailable() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let access = resolver(&registry, &[], None);
    let store: Option<&FakeStore> = None;
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, store, &registry, &access).await),
        (
            503,
            String::from("Managed agent policy requires a database")
        )
    );
}

#[rstest]
#[case::managed(MANAGED, "agent-1")]
#[case::with_an_identity(BOUND, "bound")]
#[tokio::test]
async fn a_registered_managed_agent_without_a_database_is_policy_unavailable(
    #[case] fields: &str,
    #[case] agent_id: &str,
) {
    let registry = registry(&[fields]);
    let mut auth = key(&format!(r#"{{"agent_id":"{agent_id}"}}"#));
    let access = resolver(&registry, &[], None);
    let store: Option<&FakeStore> = None;
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, store, &registry, &access).await),
        (
            503,
            String::from("Managed agent policy requires a database")
        )
    );
}

#[tokio::test]
async fn a_missing_agent_with_a_context_is_gone() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Autonomous, None));
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (403, String::from("Agent no longer exists"))
    );
}

#[tokio::test]
async fn a_retired_agent_is_gone() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    let store = FakeStore {
        agents: Vec::new(),
        retired: true,
        failure: None,
        retired_failure: None,
    };
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (403, String::from("Agent no longer exists"))
    );
}

#[tokio::test]
async fn an_unknown_agent_without_a_context_is_admitted() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
    assert!(auth.extra.managed_agent_policy.is_none());
}

#[rstest]
#[case::policy_unavailable(FailureCode::PolicyUnavailable, 503)]
#[case::identity_denied(FailureCode::IdentityDenied, 403)]
#[tokio::test]
async fn a_store_failure_answers_by_its_code(#[case] code: FailureCode, #[case] status: u16) {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    let store = FakeStore {
        agents: Vec::new(),
        retired: false,
        failure: Some(AgentIdentityFailure {
            code,
            message: String::from("Agent policy could not be loaded"),
        }),
        retired_failure: None,
    };
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (status, String::from("Agent policy could not be loaded"))
    );
}

#[rstest]
#[case::policy_unavailable(FailureCode::PolicyUnavailable, 503)]
#[case::identity_denied(FailureCode::IdentityDenied, 403)]
#[tokio::test]
async fn a_retired_failure_answers_by_its_code(#[case] code: FailureCode, #[case] status: u16) {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    let store = FakeStore {
        agents: Vec::new(),
        retired: false,
        failure: None,
        retired_failure: Some(AgentIdentityFailure {
            code,
            message: String::from("Agent history is unavailable"),
        }),
    };
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (status, String::from("Agent history is unavailable"))
    );
}

#[tokio::test]
async fn a_non_managed_agent_leaves_auth_untouched() {
    let registry = registry(&[PLAIN]);
    let mut auth = key(r#"{"agent_id":"plain"}"#);
    let mut store = FakeStore::default();
    store.agents.push(agent(PLAIN));
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
    assert!(auth.extra.managed_agent_policy.is_none());
    assert!(auth.extra.billing_agent_policy.is_none());
    assert!(!auth.extra.requires_fresh_policy);
}

#[tokio::test]
async fn claims_without_a_context_require_a_verified_identity() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    claims(&mut auth);
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (
            403,
            String::from("A managed agent requires a matching verified identity")
        )
    );
}

#[tokio::test]
async fn empty_claims_are_no_claims() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.jwt_claims = Some(Payload(BTreeMap::new()));
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (
            403,
            String::from("This agent requires its bound identity provider token")
        )
    );
}

#[tokio::test]
async fn an_admission_failure_answers_403_with_its_message() {
    let registry = registry(&[]);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let mut store = FakeStore::default();
    store.agents.push(agent(DISABLED));
    let access = resolver(&registry, &["agent-1"], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (403, String::from("Agent execution is disabled"))
    );
}

#[tokio::test]
async fn a_matching_context_marks_the_policy() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &["agent-1"], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
    assert_eq!(
        auth.extra
            .managed_agent_policy
            .as_ref()
            .map(|agent| agent.agent_id.as_str()),
        Some("agent-1")
    );
    assert!(auth.extra.billing_agent_policy.is_some());
    assert!(auth.extra.requires_fresh_policy);
}

#[tokio::test]
async fn a_delegated_context_the_human_cannot_invoke_is_refused_after_the_policy_is_marked() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &[], None);
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (
            403,
            String::from("The delegated user is not permitted to invoke this agent")
        )
    );
    assert!(auth.extra.managed_agent_policy.is_some());
    assert!(auth.extra.requires_fresh_policy);
}

#[tokio::test]
async fn a_delegated_context_the_human_holds_is_admitted() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &["agent-1"], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_autonomous_context_skips_the_grant_check() {
    let registry = registry(&[AUTONOMOUS]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Autonomous, None));
    let mut store = FakeStore::default();
    store.agents.push(agent(AUTONOMOUS));
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_verified_delegation_skips_the_grant_check() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    auth.managed_delegation_verified = true;
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &[], None);
    admit_managed_actor(&mut auth, Some(&store), &registry, &access)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_grant_failure_goes_out_with_its_status() {
    let registry = registry(&[MANAGED]);
    let mut auth = key(r#"{"agent_id":"agent-1","team_id":"team-1"}"#);
    auth.extra.managed_agent_context = Some(context(ManagedAgentMode::Delegated, Some("human")));
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = AgentAccessResolver {
        registry: Arc::clone(&registry),
        grants: FakeGrants {
            granted: ids(&["agent-1"]),
            failure: None,
        },
        targets: FakeTargets::default(),
        humans: FakeHumans { failure: Some(500) },
    };
    assert_eq!(
        refusal(admit_managed_actor(&mut auth, Some(&store), &registry, &access).await),
        (500, String::from("the case port failed"))
    );
}
