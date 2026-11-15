use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::agent_policy::admission::{actor_admission_failure, managed_agent_policy};
use crate::agent_policy::invocation::invocation_target;
use crate::agent_policy::{AgentResponse, ManagedAgentContext};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

#[derive(Clone, Copy)]
enum AgentState {
    Ready,
    Disabled,
    Inactive,
}

fn response(text: &str) -> AgentResponse {
    serde_json::from_str(text).unwrap()
}

fn agent(state: AgentState, execution_mode: &str) -> AgentResponse {
    let (enabled, active) = match state {
        AgentState::Ready => (true, true),
        AgentState::Disabled => (false, true),
        AgentState::Inactive => (true, false),
    };
    response(&format!(
        r#"{{"agent_id":"agent","agent_name":"agent","agent_card_params":{{}},"enabled":{enabled},"identity_managed":true,"execution_mode":"{execution_mode}","identity":{{"agent_id":"agent","active":{active},"provider":"microsoft_entra","tenant_id":"tenant","client_id":"client","issuer":"issuer","revision":"rev-1"}}}}"#
    ))
}

fn agent_without_identity(enabled: bool) -> AgentResponse {
    response(&format!(
        r#"{{"agent_id":"agent","agent_name":"agent","agent_card_params":{{}},"enabled":{enabled},"identity_managed":true,"identity":null}}"#
    ))
}

fn context(text: &str) -> ManagedAgentContext {
    serde_json::from_str(text).unwrap()
}

fn allowed_context() -> ManagedAgentContext {
    context(r#"{"agent_id":"agent","binding_revision":"rev-1","mode":"autonomous"}"#)
}

fn body(text: &str) -> IndexMap<String, Json> {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(fields) => fields,
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("fixture must be an object, got {other:?}");
        }
    }
}

#[test]
fn policy_accessor_returns_admitted_agent() {
    let policy = agent(AgentState::Ready, "autonomous");
    let mut auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
    auth.extra.managed_agent_policy = Some(policy);
    let admitted = managed_agent_policy(Some(&auth)).unwrap();
    assert_eq!(admitted.agent_id, "agent");
}

#[test]
fn policy_accessor_without_admission_is_none() {
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).unwrap();
    assert!(managed_agent_policy(Some(&auth)).is_none());
    assert!(managed_agent_policy(None).is_none());
}

#[test]
fn policy_actor_admission_succeeds() {
    let policy = agent(AgentState::Ready, "autonomous");
    assert!(actor_admission_failure(&policy, Some(&allowed_context())).is_none());
}

#[rstest]
#[case::disabled(AgentState::Disabled)]
#[case::inactive_identity(AgentState::Inactive)]
fn policy_actor_disabled_or_inactive_fails(#[case] state: AgentState) {
    let policy = agent(state, "autonomous");
    let failure = actor_admission_failure(&policy, Some(&allowed_context())).unwrap();
    assert_eq!(failure.message, "Agent execution is disabled");
}

#[test]
fn policy_actor_missing_identity_fails() {
    let policy = agent_without_identity(true);
    let failure = actor_admission_failure(&policy, Some(&allowed_context())).unwrap();
    assert_eq!(failure.message, "Agent execution is disabled");
}

#[test]
fn policy_actor_missing_context_fails() {
    let policy = agent(AgentState::Ready, "autonomous");
    let failure = actor_admission_failure(&policy, None).unwrap();
    assert_eq!(
        failure.message,
        "This agent requires its bound identity provider token"
    );
}

#[rstest]
#[case::other_agent(r#"{"agent_id":"other","binding_revision":"rev-1","mode":"autonomous"}"#)]
#[case::other_revision(r#"{"agent_id":"agent","binding_revision":"rev-2","mode":"autonomous"}"#)]
#[case::missing_revision(r#"{"agent_id":"agent","mode":"autonomous"}"#)]
fn policy_actor_context_mismatch_fails(#[case] input: &str) {
    let policy = agent(AgentState::Ready, "autonomous");
    let failure = actor_admission_failure(&policy, Some(&context(input))).unwrap();
    assert_eq!(
        failure.message,
        "Agent identity changed during authentication; retry"
    );
}

#[rstest]
#[case::autonomous_with_delegated("autonomous", "delegated", false)]
#[case::delegated_with_autonomous("delegated", "autonomous", false)]
#[case::both_with_autonomous("both", "autonomous", true)]
#[case::both_with_delegated("both", "delegated", true)]
fn policy_actor_mode_compatibility(
    #[case] agent_mode: &str,
    #[case] context_mode: &str,
    #[case] accepted: bool,
) {
    let policy = agent(AgentState::Ready, agent_mode);
    let admitted = context(&format!(
        r#"{{"agent_id":"agent","binding_revision":"rev-1","mode":"{context_mode}","user_id":"user"}}"#
    ));
    let result = actor_admission_failure(&policy, Some(&admitted));
    if accepted {
        assert!(result.is_none());
    } else {
        assert_eq!(
            result.unwrap().message,
            "Agent is not enabled for this execution mode"
        );
    }
}

#[rstest]
#[case::absent(r#"{"agent_id":"agent","binding_revision":"rev-1","mode":"delegated"}"#)]
#[case::empty(r#"{"agent_id":"agent","binding_revision":"rev-1","mode":"delegated","user_id":""}"#)]
fn policy_actor_delegated_requires_user_id(#[case] input: &str) {
    let policy = agent(AgentState::Ready, "delegated");
    let admitted = context(input);
    let failure = actor_admission_failure(&policy, Some(&admitted)).unwrap();
    assert_eq!(failure.message, "A verified human subject is required");
}

#[test]
fn policy_actor_delegated_with_user_id_succeeds() {
    let policy = agent(AgentState::Ready, "delegated");
    let admitted = context(
        r#"{"agent_id":"agent","binding_revision":"rev-1","mode":"delegated","user_id":"user"}"#,
    );
    assert!(actor_admission_failure(&policy, Some(&admitted)).is_none());
}

#[rstest]
#[case::plain("/a2a/agent-1", Some("agent-1"))]
#[case::prefixed("/v1/a2a/agent-1", Some("agent-1"))]
#[case::nested("/a2a/agent-1/message/send", Some("agent-1"))]
#[case::no_leading_slash("a2a/agent-1", Some("agent-1"))]
#[case::trailing_slash("/a2a/agent-1/", Some("agent-1"))]
#[case::empty_id("/a2a/", None)]
#[case::double_slash("/a2a//x", None)]
#[case::collection_only("/a2a", None)]
#[case::agents_route("/agents", None)]
#[case::v1_agents_route("/v1/agents", None)]
fn policy_invocation_target_from_path(#[case] route: &str, #[case] expected: Option<&str>) {
    let empty = body("{}");
    assert_eq!(invocation_target(route, &empty).as_deref(), expected);
}

#[rstest]
#[case::text(r#"{"model":"a2a/remote-agent"}"#, Some("remote-agent"))]
#[case::empty(r#"{"model":"a2a/"}"#, None)]
#[case::no_prefix(r#"{"model":"gpt-4"}"#, None)]
#[case::null(r#"{"model":null}"#, None)]
#[case::number(r#"{"model":5}"#, None)]
#[case::missing("{}", None)]
fn policy_invocation_target_from_model(#[case] input: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        invocation_target("/v1/chat/completions", &body(input)).as_deref(),
        expected
    );
}

#[test]
fn policy_invocation_target_prefers_path() {
    let chat_body = body(r#"{"model":"a2a/model-agent"}"#);
    assert_eq!(
        invocation_target("/v1/a2a/path-agent", &chat_body).as_deref(),
        Some("path-agent")
    );
}
