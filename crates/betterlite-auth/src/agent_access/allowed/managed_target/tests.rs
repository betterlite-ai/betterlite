#![cfg(test)]

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::agent_access::allowed::is_agent_allowed;
use crate::agent_access::error::AgentAccessError;
use crate::agent_access::tests::{
    FakeGrants, FakeHumans, FakeResolver, FakeTargets, agent, auth, calls, managed_row,
    rejected_status, resolver, restricted,
};
use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::caller::AgentCaller;
use crate::agent_policy::{AgentResponse, PolicyFlag};
use crate::agent_registry::AgentRegistryApi as _;
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::resolved_key::auth::UserAPIKeyAuth;

const PLAIN_KEY: &str = r#"{"api_key":"sk-plain","user_id":"alice"}"#;

fn managed_resolver(row: AgentResponse, grants: FakeGrants, humans: FakeHumans) -> FakeResolver {
    let mut targets = FakeTargets::default();
    targets.database = true;
    targets.agents.insert(row.agent_id.clone(), Ok(Some(row)));
    resolver(grants, targets, humans)
}

fn granting(label: &str, agent_ids: &[&str]) -> FakeGrants {
    let mut grants = FakeGrants::default();
    grants
        .key_team
        .insert(label.to_owned(), Ok(restricted(agent_ids)));
    grants
}

fn hashed(key: &UserAPIKeyAuth) -> String {
    let token = key.view.token.token.as_ref().unwrap();
    token.expose_secret().to_owned()
}

#[derive(Clone, Copy)]
enum Broken {
    Disabled,
    NoIdentity,
    InactiveIdentity,
    NoAuth,
}

#[rstest]
#[case::disabled_target(Broken::Disabled)]
#[case::target_without_identity(Broken::NoIdentity)]
#[case::inactive_identity(Broken::InactiveIdentity)]
#[case::absent_caller_auth(Broken::NoAuth)]
#[tokio::test]
async fn managed_target_is_denied_before_any_grant_is_read(#[case] broken: Broken) {
    let mut row = managed_row("bot");
    match broken {
        Broken::Disabled => row.enabled = PolicyFlag(false),
        Broken::NoIdentity => row.identity = None,
        Broken::InactiveIdentity => {
            if let Some(identity) = row.identity.as_mut() {
                identity.active = PolicyFlag(false);
            }
        }
        Broken::NoAuth => {}
    }
    let access_resolver = managed_resolver(row, granting("alice", &["bot"]), FakeHumans::default());
    let key = auth(PLAIN_KEY);
    let caller = match broken {
        Broken::NoAuth => None,
        Broken::Disabled | Broken::NoIdentity | Broken::InactiveIdentity => Some(&key),
    };
    let allowed = is_agent_allowed(&access_resolver, "bot", caller)
        .await
        .unwrap();
    assert!(!allowed);
    assert!(calls(&access_resolver.grants.calls).is_empty());
    assert!(calls(&access_resolver.humans.calls).is_empty());
}

#[tokio::test]
async fn managed_target_missing_from_the_store_is_denied_when_the_registry_says_managed() {
    let access_resolver = resolver(
        granting("alice", &["bot"]),
        FakeTargets::default(),
        FakeHumans::default(),
    );
    access_resolver.registry.register_agent(managed_row("bot"));
    let key = auth(PLAIN_KEY);
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&key))
        .await
        .unwrap();
    assert!(!allowed);
    assert_eq!(calls(&access_resolver.targets.calls), ["agent bot"]);
    assert!(calls(&access_resolver.grants.calls).is_empty());
}

#[tokio::test]
async fn managed_registry_copy_answers_from_the_store_row_without_a_database() {
    let mut targets = FakeTargets::default();
    targets
        .agents
        .insert(String::from("bot"), Ok(Some(managed_row("bot"))));
    let access_resolver = resolver(granting("alice", &["bot"]), targets, FakeHumans::default());
    access_resolver.registry.register_agent(managed_row("bot"));
    let key = auth(r#"{"user_id":"alice"}"#);
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&key))
        .await
        .unwrap();
    assert!(allowed);
    assert_eq!(
        calls(&access_resolver.grants.calls),
        [
            "key_team alice strict=true",
            "caller alice strict=true",
            "ceiling alice strict=true",
        ]
    );
}

#[rstest]
#[case::identity_denied(FailureCode::IdentityDenied, 403)]
#[case::policy_unavailable(FailureCode::PolicyUnavailable, 503)]
#[tokio::test]
async fn managed_store_failure_is_an_identity_error(
    #[case] code: FailureCode,
    #[case] status: u16,
) {
    let mut targets = FakeTargets::default();
    targets.database = true;
    let failure = AgentIdentityFailure {
        code,
        message: String::from("Agent policy could not be loaded"),
    };
    targets.agents.insert(String::from("bot"), Err(failure));
    let access_resolver = resolver(FakeGrants::default(), targets, FakeHumans::default());
    let outcome = is_agent_allowed(&access_resolver, "bot", None).await;
    assert!(matches!(&outcome, Err(AgentAccessError::Identity(_))));
    assert_eq!(rejected_status(outcome), status);
}

#[tokio::test]
async fn managed_target_reloads_a_plain_key_and_answers_from_its_fresh_strict_grants() {
    let mut original = auth(PLAIN_KEY);
    original.extra.agent_caller = Some(AgentCaller {
        user_id: Some(String::from("human")),
        team_id: Some(String::from("humans")),
    });
    let mut reloaded = auth(r#"{"user_id":"reloaded"}"#);
    reloaded.extra.agent_caller = None;
    let mut humans = FakeHumans::default();
    humans.reloaded_key = Ok(reloaded);
    let access_resolver = managed_resolver(
        managed_row("bot"),
        granting("reloaded", &["bot", "other"]),
        humans,
    );
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&original))
        .await
        .unwrap();
    assert!(allowed);
    assert_eq!(
        calls(&access_resolver.humans.calls),
        [format!(
            "reload_key {} check_db_only=true",
            hashed(&original)
        )]
    );
    let seen = context_auth(&access_resolver);
    assert_eq!(seen.view.token.user_id.as_deref(), Some("reloaded"));
    assert!(seen.extra.requires_fresh_policy);
    let caller = seen.extra.agent_caller.as_ref().unwrap();
    assert_eq!(caller.user_id.as_deref(), Some("human"));
    assert_eq!(caller.team_id.as_deref(), Some("humans"));
    assert_eq!(
        calls(&access_resolver.grants.calls)[0],
        "key_team reloaded strict=true"
    );
}

fn context_auth(access_resolver: &FakeResolver) -> UserAPIKeyAuth {
    let seen = access_resolver.humans.context_auths.lock().unwrap();
    assert_eq!(seen.len(), 1);
    seen[0].clone()
}

#[tokio::test]
async fn managed_target_falls_back_to_the_token_when_there_is_no_api_key() {
    let key = auth(r#"{"token":"hashed-token","user_id":"alice"}"#);
    let mut humans = FakeHumans::default();
    humans.reloaded_key = Ok(auth(r#"{"user_id":"alice"}"#));
    let access_resolver = managed_resolver(managed_row("bot"), granting("alice", &["bot"]), humans);
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&key))
        .await
        .unwrap();
    assert!(allowed);
    assert_eq!(
        calls(&access_resolver.humans.calls),
        ["reload_key hashed-token check_db_only=true"]
    );
}

#[derive(Clone, Copy)]
enum GivenAuthority {
    SessionToken,
    CustomAuth,
    ManagedKey,
    NoKeyHash,
}

#[rstest]
#[case::session_token(GivenAuthority::SessionToken)]
#[case::custom_auth(GivenAuthority::CustomAuth)]
#[case::managed_key(GivenAuthority::ManagedKey)]
#[case::no_key_hash(GivenAuthority::NoKeyHash)]
#[tokio::test]
async fn managed_target_uses_the_auth_as_given(#[case] given: GivenAuthority) {
    let mut key = match given {
        GivenAuthority::NoKeyHash => auth(r#"{"user_id":"alice"}"#),
        GivenAuthority::SessionToken | GivenAuthority::CustomAuth | GivenAuthority::ManagedKey => {
            auth(PLAIN_KEY)
        }
    };
    match given {
        GivenAuthority::SessionToken => key.extra.is_session_token = true,
        GivenAuthority::CustomAuth => key.extra.provenance.authenticated_by_custom_auth = true,
        GivenAuthority::ManagedKey => {
            key.extra.managed_agent_policy = Some(agent(
                r#"{"agent_id":"caller-bot","agent_name":"caller-bot","agent_card_params":{},"object_permission":{"object_permission_id":"op","agents":["bot"]}}"#,
            ));
        }
        GivenAuthority::NoKeyHash => {}
    }
    let mut grants = granting("alice", &["bot"]);
    grants.key_strict = Ok(restricted(&["bot"]));
    let mut humans = FakeHumans::default();
    humans.reloaded_key = Ok(auth(r#"{"user_id":"reloaded"}"#));
    let access_resolver = managed_resolver(managed_row("bot"), grants, humans);
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&key))
        .await
        .unwrap();
    assert!(allowed);
    assert!(calls(&access_resolver.humans.calls).is_empty());
    let seen = context_auth(&access_resolver);
    assert_eq!(seen.view.token.user_id.as_deref(), Some("alice"));
    assert!(seen.extra.requires_fresh_policy);
}

#[rstest]
#[case::unrestricted_grants(AgentAccess::Unrestricted)]
#[case::other_agents_only(restricted(&["other"]))]
#[tokio::test]
async fn managed_target_needs_an_explicit_grant(#[case] access: AgentAccess) {
    let mut grants = FakeGrants::default();
    grants.key_team.insert(String::from("alice"), Ok(access));
    let access_resolver = managed_resolver(managed_row("bot"), grants, FakeHumans::default());
    let key = auth(r#"{"user_id":"alice"}"#);
    let allowed = is_agent_allowed(&access_resolver, "bot", Some(&key))
        .await
        .unwrap();
    assert!(!allowed);
}

#[tokio::test]
async fn managed_target_key_reload_failure_propagates() {
    let access_resolver = managed_resolver(
        managed_row("bot"),
        granting("alice", &["bot"]),
        FakeHumans::default(),
    );
    let key = auth(PLAIN_KEY);
    let outcome = is_agent_allowed(&access_resolver, "bot", Some(&key)).await;
    assert_eq!(rejected_status(outcome), 401);
}
