use betterlite_auth::agent_policy::AgentResponse;
use betterlite_core::api::request_destination::RequestData;

use super::agent_registry::AgentPolicyRegistry;
use super::descriptor::rate_limit_descriptor;
use super::session_resolution::session_id_from_data;
use crate::limiter_records::RateLimitDescriptor;

mod tests;

pub struct AgentDescriptorBuilder<'registry> {
    pub window_size: i64,
    pub registry: &'registry dyn AgentPolicyRegistry,
}

impl AgentDescriptorBuilder<'_> {
    // LiteLLM's _create_agent_rate_limit_descriptors: one agent-wide bucket and one per session, when the policy sets them
    #[must_use]
    pub fn create_agent_descriptors(
        &self,
        agent_id: &str,
        data: &RequestData,
        policy: Option<&AgentResponse>,
    ) -> Vec<RateLimitDescriptor> {
        let Some(agent) = policy
            .cloned()
            .or_else(|| self.registry.agent_by_id(agent_id))
        else {
            return Vec::new();
        };
        let mut descriptors = Vec::new();
        if agent.rpm_limit.is_some() || agent.tpm_limit.is_some() {
            descriptors.push(rate_limit_descriptor(
                "agent",
                agent_id.to_owned(),
                agent.rpm_limit,
                agent.tpm_limit,
                self.window_size,
            ));
        }
        let resolved_session = session_id_from_data(data);
        if let Some(session_id) = resolved_session
            && (agent.session_rpm_limit.is_some() || agent.session_tpm_limit.is_some())
        {
            descriptors.push(rate_limit_descriptor(
                "agent_session",
                format!("{agent_id}:{session_id}"),
                agent.session_rpm_limit,
                agent.session_tpm_limit,
                self.window_size,
            ));
        }
        descriptors
    }
}
