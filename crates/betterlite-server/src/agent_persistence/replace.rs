use betterlite_core::agents::AgentConfig;
use betterlite_core::agents::PatchAgentRequest;

pub(super) fn patch_request(agent: &AgentConfig) -> PatchAgentRequest {
    PatchAgentRequest {
        identity: agent.identity.clone(),
        enabled: agent.enabled,
        execution_mode: agent.execution_mode.clone(),
        agent_name: Some(agent.agent_name.clone()),
        agent_card_params: agent.agent_card_params.clone(),
        litellm_params: agent.litellm_params.clone(),
        object_permission: agent.object_permission.clone(),
        tpm_limit: agent.tpm_limit,
        rpm_limit: agent.rpm_limit,
        session_tpm_limit: agent.session_tpm_limit,
        session_rpm_limit: agent.session_rpm_limit,
        static_headers: agent.static_headers.clone(),
        extra_headers: agent.extra_headers.clone(),
        access_group_ids: agent.access_group_ids.clone(),
        kill_switch: agent.kill_switch.clone(),
    }
}

mod tests;
