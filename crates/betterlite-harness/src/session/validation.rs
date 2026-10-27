use thiserror::Error;

use crate::errors::HarnessError;
use crate::options::OptionsKind as _;
use crate::session::SessionConfig;
use crate::transformation::BaseHarnessConfig;
use crate::types::{Capabilities, Capability, Harness, PermissionMode};

pub const PERMISSION_MODES: &[PermissionMode] = &[
    PermissionMode::Full,
    PermissionMode::ReadOnly,
    PermissionMode::Ask,
    PermissionMode::Edit,
];

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("max_turns must be >= 1")]
    MaxTurns,
    #[error("timeout must be > 0")]
    Timeout,
    #[error("install=True is not supported yet; put the runtime binary on PATH in the sandbox")]
    InstallUnsupported,
    #[error("Harness.{} does not support permissions={permission:?}; supported: {supported:?}", .harness.name())]
    PermissionUnsupported {
        harness: Harness,
        permission: PermissionMode,
        supported: Vec<PermissionMode>,
    },
    #[error("permissions='ask' needs on_approval=, or use stream() and answer Approval events")]
    ApprovalRequired,
    #[error("{0}")]
    Harness(#[from] HarnessError),
}

pub fn check_basic<S>(config: &SessionConfig<S>) -> Result<(), ValidationError> {
    let settings = &config.parameters.settings;
    if settings.max_turns == Some(0) {
        return Err(ValidationError::MaxTurns);
    }
    if settings.timeout.is_some_and(|timeout| timeout.is_zero()) {
        return Err(ValidationError::Timeout);
    }
    if config.parameters.install {
        return Err(ValidationError::InstallUnsupported);
    }
    Ok(())
}

pub fn check_options<S, C>(config: &SessionConfig<S>) -> Result<(), ValidationError>
where
    C: BaseHarnessConfig,
{
    match &config.parameters.settings.options {
        Some(options) if C::Options::pick(options).is_none() => {
            Err(ValidationError::Harness(HarnessError::OptionsMismatch {
                given: options.name(),
                harness: config.harness,
                expected: C::Options::NAME,
            }))
        }
        Some(_) | None => Ok(()),
    }
}

pub fn check_capabilities<S>(
    config: &SessionConfig<S>,
    capabilities: &Capabilities,
    interactive: bool,
) -> Result<(), ValidationError> {
    let settings = &config.parameters.settings;
    let permission = settings.permissions;
    if !capabilities.permission_modes.contains(&permission) {
        return Err(ValidationError::PermissionUnsupported {
            harness: config.harness,
            permission,
            supported: capabilities.permission_modes.iter().copied().collect(),
        });
    }
    let required = [
        (permission == PermissionMode::Ask, Capability::ToolApproval),
        (
            settings.output_schema.is_some(),
            Capability::StructuredOutput,
        ),
        (!settings.skills.is_empty(), Capability::Skills),
        (
            !settings.disable_tools.is_empty(),
            Capability::ToolFiltering,
        ),
    ];
    for (enabled, capability) in required {
        if enabled && !capabilities.supported.contains(&capability) {
            return Err(ValidationError::Harness(
                HarnessError::CapabilityUnsupported {
                    harness: config.harness,
                    capability,
                },
            ));
        }
        // LiteLLM asks for the handler right after the approval capability, before the other capabilities
        if enabled
            && capability == Capability::ToolApproval
            && settings.on_approval.is_none()
            && !interactive
        {
            return Err(ValidationError::ApprovalRequired);
        }
    }
    Ok(())
}

pub fn validate<S, C>(
    config: &SessionConfig<S>,
    harness_config: &C,
    interactive: bool,
) -> Result<(), ValidationError>
where
    C: BaseHarnessConfig,
{
    check_basic(config)?;
    check_options::<S, C>(config)?;
    check_capabilities(config, &harness_config.capabilities(), interactive)
}
