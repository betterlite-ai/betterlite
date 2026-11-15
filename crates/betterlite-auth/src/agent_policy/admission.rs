use crate::agent_policy::{
    AgentExecutionMode, AgentResponse, ManagedAgentContext, ManagedAgentMode,
};
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::resolved_key::auth::UserAPIKeyAuth;

#[must_use]
pub const fn managed_agent_policy(auth: Option<&UserAPIKeyAuth>) -> Option<&AgentResponse> {
    if let Some(credentials) = auth {
        return credentials.extra.managed_agent_policy.as_ref();
    }
    None
}

#[must_use]
pub fn actor_admission_failure(
    agent: &AgentResponse,
    context: Option<&ManagedAgentContext>,
) -> Option<AgentIdentityFailure> {
    let enabled_identity = agent
        .identity
        .as_ref()
        .filter(|identity| identity.active.0 && agent.enabled.0);
    let Some(identity) = enabled_identity else {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Agent execution is disabled"),
        });
    };
    let Some(admitted) = context else {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("This agent requires its bound identity provider token"),
        });
    };
    let identity_changed = admitted.agent_id != agent.agent_id
        || admitted.binding_revision.as_deref() != Some(identity.revision.as_str());
    if identity_changed {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Agent identity changed during authentication; retry"),
        });
    }
    if !execution_mode_matches(&agent.execution_mode, &admitted.mode) {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Agent is not enabled for this execution mode"),
        });
    }
    if admitted.mode == ManagedAgentMode::Delegated
        && admitted.user_id.as_deref().is_none_or(str::is_empty)
    {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("A verified human subject is required"),
        });
    }
    None
}

const fn execution_mode_matches(
    execution_mode: &AgentExecutionMode,
    mode: &ManagedAgentMode,
) -> bool {
    match (execution_mode, mode) {
        (AgentExecutionMode::Both, ManagedAgentMode::Autonomous | ManagedAgentMode::Delegated)
        | (AgentExecutionMode::Autonomous, ManagedAgentMode::Autonomous)
        | (AgentExecutionMode::Delegated, ManagedAgentMode::Delegated) => true,
        (AgentExecutionMode::Autonomous, ManagedAgentMode::Delegated)
        | (AgentExecutionMode::Delegated, ManagedAgentMode::Autonomous) => false,
    }
}
