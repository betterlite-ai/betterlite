#![cfg(test)]

use std::str::FromStr as _;
use std::sync::Arc;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::agent_access::resolver::AgentAccessResolver;
use crate::agent_policy::AgentResponse;
use crate::identity::{AgentIdentityFailure, FailureCode};

use super::prepare_agent_invocation;
use crate::managed_admission::ports::InvocationSources;
use crate::managed_admission::tests::{
    FakeGrants, FakeHumans, FakeStore, FakeTargets, MANAGED, MissedReadThrough, PLAIN,
    RegisteringReadThrough, agent, ids, key, refusal, registry, resolver,
};

const PLAIN_COPY: &str = r#"{"agent_id":"agent-1","agent_name":"stale","agent_card_params":{}}"#;
const STALE_ID: &str = r#"{"agent_id":"id-1","agent_name":"card-name","agent_card_params":{},"identity_managed":true,"identity":{"agent_id":"id-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;
const IDENTITY_ONLY: &str = r#"{"agent_id":"bound","agent_name":"bound-agent","agent_card_params":{},"identity":{"agent_id":"bound","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#;

fn priced(params: Option<&str>) -> AgentResponse {
    let mut fields = String::from(
        r#"{"agent_id":"agent-1","agent_name":"agent-1","agent_card_params":{},"identity_managed":true,"identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}"#,
    );
    if let Some(params) = params {
        fields.push_str(r#","litellm_params":"#);
        fields.push_str(params);
    }
    fields.push('}');
    agent(&fields)
}

#[tokio::test]
async fn an_unknown_target_is_ignored_after_one_read_through_attempt() {
    let registry = registry(&[]);
    let through = MissedReadThrough::default();
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let store: Option<&FakeStore> = None;
    let sources = InvocationSources {
        store,
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "ghost", true, &sources, &access)
        .await
        .unwrap();
    assert_eq!(through.attempts.lock().unwrap().clone(), vec!["ghost"]);
    assert!(auth.extra.invoked_agent_id.is_none());
}

#[tokio::test]
async fn a_read_through_that_changed_the_registry_is_looked_up_again() {
    let registry = registry(&[]);
    let through = RegisteringReadThrough {
        registry: Arc::clone(&registry),
        agent: agent(MANAGED),
    };
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let store: Option<&FakeStore> = None;
    let sources = InvocationSources {
        store,
        read_through: &through,
    };
    assert_eq!(
        refusal(prepare_agent_invocation(&mut auth, "agent-1", true, &sources, &access).await),
        (
            503,
            String::from("Managed agent policy requires a database")
        )
    );
}

#[tokio::test]
async fn a_registry_agent_found_by_name_is_prepared() {
    let registry = registry(&[STALE_ID]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(agent(STALE_ID));
    let access = resolver(&registry, &["id-1"], Some(agent(STALE_ID)));
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "card-name", true, &sources, &access)
        .await
        .unwrap();
    assert_eq!(auth.extra.invoked_agent_id.as_deref(), Some("id-1"));
    assert!(through.attempts.lock().unwrap().is_empty());
}

#[rstest]
#[case::managed(MANAGED, "managed-agent")]
#[case::with_an_identity(IDENTITY_ONLY, "bound-agent")]
#[tokio::test]
async fn a_managed_target_without_a_database_is_policy_unavailable(
    #[case] fields: &str,
    #[case] target: &str,
) {
    let registry = registry(&[fields]);
    let through = MissedReadThrough::default();
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let store: Option<&FakeStore> = None;
    let sources = InvocationSources {
        store,
        read_through: &through,
    };
    assert_eq!(
        refusal(prepare_agent_invocation(&mut auth, target, true, &sources, &access).await),
        (
            503,
            String::from("Managed agent policy requires a database")
        )
    );
}

#[tokio::test]
async fn a_plain_target_without_a_database_is_prepared_without_changes() {
    let registry = registry(&[PLAIN]);
    let through = MissedReadThrough::default();
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let store: Option<&FakeStore> = None;
    let sources = InvocationSources {
        store,
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "plain-agent", true, &sources, &access)
        .await
        .unwrap();
    assert!(auth.extra.invoked_agent_id.is_none());
    assert!(auth.extra.agent_invocation_cost.is_none());
}

#[tokio::test]
async fn a_managed_target_the_store_lost_is_gone() {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    assert_eq!(
        refusal(
            prepare_agent_invocation(&mut auth, "managed-agent", true, &sources, &access).await
        ),
        (403, String::from("Invoked agent no longer exists"))
    );
}

#[tokio::test]
async fn a_plain_target_the_store_lost_falls_back_to_the_registry() {
    let registry = registry(&[PLAIN]);
    let through = MissedReadThrough::default();
    let store = FakeStore::default();
    let access = resolver(&registry, &[], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "plain-agent", true, &sources, &access)
        .await
        .unwrap();
    assert!(auth.extra.invoked_agent_id.is_none());
}

#[rstest]
#[case::policy_unavailable(FailureCode::PolicyUnavailable, 503)]
#[case::identity_denied(FailureCode::IdentityDenied, 403)]
#[tokio::test]
async fn a_store_failure_answers_by_its_code(#[case] code: FailureCode, #[case] status: u16) {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
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
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    assert_eq!(
        refusal(
            prepare_agent_invocation(&mut auth, "managed-agent", true, &sources, &access).await
        ),
        (status, String::from("Agent policy could not be loaded"))
    );
}

#[tokio::test]
async fn the_store_row_decides_over_the_registry_copy() {
    let registry = registry(&[PLAIN_COPY]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(priced(Some(r#"{"cost_per_query":2}"#)));
    let access = resolver(&registry, &["agent-1"], Some(priced(None)));
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "agent-1", true, &sources, &access)
        .await
        .unwrap();
    assert_eq!(auth.extra.invoked_agent_id.as_deref(), Some("agent-1"));
    assert!(auth.extra.billing_agent_policy.is_some());
    assert_eq!(auth.extra.agent_invocation_cost, Some(Decimal::TWO));
}

#[tokio::test]
async fn a_managed_key_calling_a_plain_agent_is_checked() {
    let registry = registry(&[PLAIN]);
    let through = MissedReadThrough::default();
    let store: Option<&FakeStore> = None;
    let access = resolver(&registry, &[], Some(agent(PLAIN)));
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.managed_agent_policy = Some(agent(MANAGED));
    let sources = InvocationSources {
        store,
        read_through: &through,
    };
    assert_eq!(
        refusal(prepare_agent_invocation(&mut auth, "plain-agent", true, &sources, &access).await),
        (
            403,
            String::from("The caller is not permitted to invoke this agent")
        )
    );
    assert!(auth.extra.invoked_agent_id.is_none());
}

#[tokio::test]
async fn a_managed_key_keeps_its_own_billing_policy() {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = resolver(&registry, &["agent-1"], None);
    let mut auth = key(r#"{"agent_id":"agent-1"}"#);
    auth.extra.managed_agent_policy = Some(agent(
        r#"{"agent_id":"agent-1","agent_name":"managed-agent","agent_card_params":{},"identity_managed":true,"object_permission":{"agents":["agent-1"]},"identity":{"agent_id":"agent-1","provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}"#,
    ));
    auth.extra.billing_agent_policy = Some(agent(PLAIN));
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "managed-agent", true, &sources, &access)
        .await
        .unwrap();
    assert_eq!(auth.extra.invoked_agent_id.as_deref(), Some("agent-1"));
    assert_eq!(
        auth.extra
            .billing_agent_policy
            .as_ref()
            .map(|agent| agent.agent_id.as_str()),
        Some("plain")
    );
}

#[tokio::test]
async fn a_grant_failure_goes_out_with_its_status() {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(agent(MANAGED));
    let access = AgentAccessResolver {
        registry: Arc::clone(&registry),
        grants: FakeGrants {
            granted: ids(&[]),
            failure: Some(500),
        },
        targets: FakeTargets {
            agent: Some(agent(MANAGED)),
            registry: None,
        },
        humans: FakeHumans::default(),
    };
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    assert_eq!(
        refusal(
            prepare_agent_invocation(&mut auth, "managed-agent", true, &sources, &access).await
        ),
        (500, String::from("the case port failed"))
    );
}

#[tokio::test]
async fn a_price_is_not_read_when_not_billable() {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(priced(Some(r#"{"cost_per_query":-1}"#)));
    let access = resolver(&registry, &["agent-1"], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "agent-1", false, &sources, &access)
        .await
        .unwrap();
    assert_eq!(auth.extra.agent_invocation_cost, Some(Decimal::ZERO));
}

#[rstest]
#[case::number(Some(r#"{"cost_per_query":2.5}"#), "2.5")]
#[case::string(Some(r#"{"cost_per_query":"0.5"}"#), "0.5")]
#[case::scientific_string(Some(r#"{"cost_per_query":"1e3"}"#), "1000")]
#[case::zero(Some(r#"{"cost_per_query":0}"#), "0")]
#[case::no_params(None, "0")]
#[case::params_without_the_key(Some("{}"), "0")]
#[tokio::test]
async fn a_valid_price_is_read_from_the_effective_agent(
    #[case] params: Option<&str>,
    #[case] expected: &str,
) {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(priced(params));
    let access = resolver(&registry, &["agent-1"], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    prepare_agent_invocation(&mut auth, "agent-1", true, &sources, &access)
        .await
        .unwrap();
    assert_eq!(
        auth.extra.agent_invocation_cost,
        Some(Decimal::from_str(expected).unwrap())
    );
}

#[rstest]
#[case::negative(r#"{"cost_per_query":-1}"#)]
#[case::null(r#"{"cost_per_query":null}"#)]
#[case::a_bool(r#"{"cost_per_query":true}"#)]
#[case::a_list(r#"{"cost_per_query":[1]}"#)]
#[case::an_object(r#"{"cost_per_query":{"amount":1}}"#)]
#[case::nan(r#"{"cost_per_query":"nan"}"#)]
#[case::infinity(r#"{"cost_per_query":"inf"}"#)]
#[case::not_a_number(r#"{"cost_per_query":"abc"}"#)]
#[case::out_of_range(r#"{"cost_per_query":"1e40"}"#)]
#[tokio::test]
async fn an_invalid_price_is_policy_unavailable(#[case] params: &str) {
    let registry = registry(&[MANAGED]);
    let through = MissedReadThrough::default();
    let mut store = FakeStore::default();
    store.agents.push(priced(Some(params)));
    let access = resolver(&registry, &["agent-1"], None);
    let mut auth = key(r#"{"user_id":"human"}"#);
    auth.extra.agent_invocation_cost = Some(Decimal::ONE);
    let sources = InvocationSources {
        store: Some(&store),
        read_through: &through,
    };
    assert_eq!(
        refusal(prepare_agent_invocation(&mut auth, "agent-1", true, &sources, &access).await),
        (503, String::from("Agent invocation price is invalid"))
    );
    assert_eq!(auth.extra.agent_invocation_cost, Some(Decimal::ONE));
}
