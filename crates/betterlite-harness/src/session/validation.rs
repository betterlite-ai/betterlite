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

pub fn check_options<S, C>(
    config: &SessionConfig<S>,
    _harness_config: &C,
) -> Result<(), ValidationError>
where
    C: BaseHarnessConfig,
{
    let Some(options) = &config.parameters.settings.options else {
        return Ok(());
    };
    if C::Options::pick(options).is_none() {
        return Err(ValidationError::Harness(HarnessError::OptionsMismatch {
            given: options.name(),
            harness: config.harness,
            expected: C::Options::NAME,
        }));
    }
    Ok(())
}

pub fn check_capabilities<S>(
    config: &SessionConfig<S>,
    capabilities: &Capabilities,
    interactive: bool,
) -> Result<(), ValidationError> {
    let settings = &config.parameters.settings;
    if !capabilities
        .permission_modes
        .contains(&settings.permissions)
    {
        return Err(ValidationError::PermissionUnsupported {
            harness: config.harness,
            permission: settings.permissions,
            supported: capabilities.permission_modes.iter().copied().collect(),
        });
    }
    check_approval(config, capabilities, interactive)?;
    let required = [
        (
            settings.output_schema.is_some(),
            Capability::StructuredOutput,
        ),
        (!config.parameters.tools.is_empty(), Capability::CustomTools),
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
    }
    Ok(())
}

fn check_approval<S>(
    config: &SessionConfig<S>,
    capabilities: &Capabilities,
    interactive: bool,
) -> Result<(), ValidationError> {
    let settings = &config.parameters.settings;
    if settings.permissions == PermissionMode::Ask {
        if !capabilities.supported.contains(&Capability::ToolApproval) {
            return Err(ValidationError::Harness(
                HarnessError::CapabilityUnsupported {
                    harness: config.harness,
                    capability: Capability::ToolApproval,
                },
            ));
        }
        if settings.on_approval.is_none() && !interactive {
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
    check_options(config, harness_config)?;
    check_capabilities(config, &harness_config.capabilities(), interactive)
}
