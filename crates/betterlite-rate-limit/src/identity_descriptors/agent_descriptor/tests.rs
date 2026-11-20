#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_core::api::request_destination::RequestData;

use crate::identity_descriptors::agent_registry::AgentPolicyRegistry;
use crate::limiter_records::{RateLimitDescriptor, RateLimitDescriptorRateLimit};

use super::AgentDescriptorBuilder;

struct Fixed(BTreeMap<String, AgentResponse>);

impl AgentPolicyRegistry for Fixed {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0.get(agent_id).cloned()
    }
}

#[derive(serde::Serialize)]
struct PolicyWire {
    agent_id: &'static str,
    agent_name: &'static str,
    agent_card_params: BTreeMap<String, String>,
    rpm_limit: Option<i64>,
    tpm_limit: Option<i64>,
    session_rpm_limit: Option<i64>,
    session_tpm_limit: Option<i64>,
}

fn policy(limits: [Option<i64>; 4]) -> AgentResponse {
    let [rpm_limit, tpm_limit, session_rpm_limit, session_tpm_limit] = limits;
    let wire = serde_json::to_string(&PolicyWire {
        agent_id: "agent-1",
        agent_name: "agent-one",
        agent_card_params: BTreeMap::new(),
        rpm_limit,
        tpm_limit,
        session_rpm_limit,
        session_tpm_limit,
    })
    .expect("the policy fixture serializes");
    serde_json::from_str(&wire).expect("the policy fixture decodes")
}

fn registry(agent: AgentResponse) -> Fixed {
    Fixed(BTreeMap::from([("agent-1".to_owned(), agent)]))
}

fn data(wire: &str) -> RequestData {
    serde_json::from_str(wire).expect("the request body fixture decodes")
}

fn descriptor(
    key: &str,
    value: &str,
    requests: Option<i64>,
    tokens: Option<i64>,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: requests,
            tokens_per_unit: tokens,
            max_parallel_requests: None,
            window_size: Some(60),
        }),
    }
}

fn built(registry: &Fixed, policy: Option<&AgentResponse>, wire: &str) -> Vec<RateLimitDescriptor> {
    let builder = AgentDescriptorBuilder {
        window_size: 60,
        registry,
    };
    builder.create_agent_descriptors("agent-1", &data(wire), policy)
}

#[test]
fn builds_the_agent_global_descriptor() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([Some(10), Some(20), None, None]);
    assert_eq!(
        built(&registry, Some(&policy), "{}"),
        vec![descriptor("agent", "agent-1", Some(10), Some(20))]
    );
}

#[test]
fn keeps_an_absent_agent_tpm() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([Some(10), None, None, None]);
    assert_eq!(
        built(&registry, Some(&policy), "{}"),
        vec![descriptor("agent", "agent-1", Some(10), None)]
    );
}

#[test]
fn adds_no_agent_descriptor_without_global_limits() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, None, None]);
    assert!(built(&registry, Some(&policy), "{}").is_empty());
}

#[test]
fn looks_up_the_policy_in_the_registry() {
    let registry = registry(policy([Some(7), None, None, None]));
    assert_eq!(
        built(&registry, None, "{}"),
        vec![descriptor("agent", "agent-1", Some(7), None)]
    );
}

#[test]
fn returns_empty_when_the_registry_has_no_agent() {
    let registry = Fixed(BTreeMap::new());
    assert!(built(&registry, None, "{}").is_empty());
}

#[test]
fn prefers_a_supplied_policy_over_the_registry() {
    let registry = registry(policy([Some(9), None, None, None]));
    let policy = policy([Some(1), None, None, None]);
    assert_eq!(
        built(&registry, Some(&policy), "{}"),
        vec![descriptor("agent", "agent-1", Some(1), None)]
    );
}

#[test]
fn adds_the_session_descriptor() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, Some(5), None]);
    assert_eq!(
        built(&registry, Some(&policy), r#"{"litellm_session_id": "s1"}"#),
        vec![descriptor("agent_session", "agent-1:s1", Some(5), None)]
    );
}

#[test]
fn carries_both_session_limits() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, Some(5), Some(6)]);
    assert_eq!(
        built(&registry, Some(&policy), r#"{"litellm_session_id": "s1"}"#),
        vec![descriptor("agent_session", "agent-1:s1", Some(5), Some(6))]
    );
}

#[test]
fn adds_no_session_descriptor_without_a_session_id() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, Some(5), Some(6)]);
    assert!(built(&registry, Some(&policy), "{}").is_empty());
}

#[test]
fn adds_no_session_descriptor_without_session_limits() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, None, None]);
    assert!(built(&registry, Some(&policy), r#"{"litellm_session_id": "s1"}"#).is_empty());
}

#[test]
fn adds_the_agent_and_session_descriptors_in_order() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([Some(10), None, Some(5), None]);
    assert_eq!(
        built(&registry, Some(&policy), r#"{"litellm_session_id": "s1"}"#),
        vec![
            descriptor("agent", "agent-1", Some(10), None),
            descriptor("agent_session", "agent-1:s1", Some(5), None),
        ]
    );
}

#[test]
fn resolves_the_session_id_from_the_request() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, Some(5), None]);
    let wire = r#"{
        "metadata": {"session_id": "meta-s"},
        "litellm_metadata": {"session_id": "llm-s"}
    }"#;
    assert_eq!(
        built(&registry, Some(&policy), wire),
        vec![descriptor("agent_session", "agent-1:meta-s", Some(5), None)]
    );
}

#[test]
fn keeps_a_zero_session_limit() {
    let registry = Fixed(BTreeMap::new());
    let policy = policy([None, None, Some(0), None]);
    assert_eq!(
        built(&registry, Some(&policy), r#"{"litellm_session_id": "s1"}"#),
        vec![descriptor("agent_session", "agent-1:s1", Some(0), None)]
    );
}
