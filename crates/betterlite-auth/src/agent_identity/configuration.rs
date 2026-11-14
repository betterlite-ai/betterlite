use crate::agent_policy::{AgentExecutionMode, AgentIdentityBinding};
use crate::identity::{AgentIdentityFailure, EntraIdentityConfig, FailureCode};

#[derive(Clone, Copy)]
pub enum IdentitySource<'identity> {
    Configuration(&'identity EntraIdentityConfig),
    Binding(&'identity AgentIdentityBinding),
}

impl IdentitySource<'_> {
    const fn has_service_principal(self) -> bool {
        match self {
            Self::Configuration(configuration) => configuration.service_principal_id.is_some(),
            Self::Binding(binding) => binding.service_principal_id.is_some(),
        }
    }

    const fn is_inactive_binding(self) -> bool {
        match self {
            Self::Configuration(_) => false,
            Self::Binding(binding) => !binding.active.0,
        }
    }
}

#[must_use]
pub fn configuration_failure(
    identity: Option<IdentitySource<'_>>,
    mode: &AgentExecutionMode,
    enabling_without_binding: bool,
) -> Option<AgentIdentityFailure> {
    if let Some(source) = identity
        && *mode != AgentExecutionMode::Delegated
        && !source.has_service_principal()
    {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(
                "Autonomous mode requires the Enterprise application service-principal object ID",
            ),
        });
    }
    if enabling_without_binding && identity.is_none_or(IdentitySource::is_inactive_binding) {
        return Some(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Bind an identity before enabling this managed agent"),
        });
    }
    None
}
