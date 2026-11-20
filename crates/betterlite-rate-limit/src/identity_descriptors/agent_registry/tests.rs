#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi};

use super::AgentPolicyRegistry;

fn agent(agent_id: &str) -> AgentResponse {
    let wire = format!(
        r#"{{"agent_id":"{agent_id}","agent_name":"name-{agent_id}","agent_card_params":{{}}}}"#
    );
    serde_json::from_str(&wire).expect("the agent fixture decodes")
}

struct Fixed(BTreeMap<String, AgentResponse>);

impl AgentPolicyRegistry for Fixed {
    fn agent_by_id(&self, agent_id: &str) -> Option<AgentResponse> {
        self.0.get(agent_id).cloned()
    }
}

#[test]
fn reads_an_agent_registered_in_the_registry() {
    let registry = AgentRegistry::default();
    AgentRegistryApi::register_agent(&registry, agent("agent-1"));
    let found =
        AgentPolicyRegistry::agent_by_id(&registry, "agent-1").expect("the registered agent");
    assert_eq!(found.agent_id, "agent-1");
    assert_eq!(found.agent_name, "name-agent-1");
}

#[test]
fn returns_none_for_an_unknown_agent_id() {
    let registry = AgentRegistry::default();
    AgentRegistryApi::register_agent(&registry, agent("agent-1"));
    assert!(AgentPolicyRegistry::agent_by_id(&registry, "agent-2").is_none());
}

#[test]
fn returns_the_policy_limits_of_the_registered_agent() {
    let registry = AgentRegistry::default();
    let mut policy = agent("agent-1");
    policy.rpm_limit = Some(10);
    policy.session_tpm_limit = Some(20);
    AgentRegistryApi::register_agent(&registry, policy);
    let found =
        AgentPolicyRegistry::agent_by_id(&registry, "agent-1").expect("the registered agent");
    assert_eq!(found.rpm_limit, Some(10));
    assert_eq!(found.session_tpm_limit, Some(20));
    assert_eq!(found.tpm_limit, None);
}

#[test]
fn a_case_supplied_registry_answers_by_id() {
    let registry = Fixed(BTreeMap::from([("agent-9".to_owned(), agent("agent-9"))]));
    let found = registry.agent_by_id("agent-9").expect("the mapped agent");
    assert_eq!(found.agent_id, "agent-9");
    assert!(registry.agent_by_id("agent-1").is_none());
}
