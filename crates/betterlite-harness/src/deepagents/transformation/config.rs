use std::collections::BTreeSet;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::options::{DeepAgentsOptions, OptionsKind as _};
use crate::transformation::BaseHarnessConfig;
use crate::types::{Capabilities, Capability, Harness, PermissionMode};

use super::require_model;
use super::{BASE_RECURSION_LIMIT, DEEPAGENTS_STEPS_PER_TURN, DEFAULT_RECURSION_LIMIT};

#[derive(Clone, Copy, Debug, Default)]
pub struct DeepAgentsHarnessConfig;

impl BaseHarnessConfig for DeepAgentsHarnessConfig {
    type Options = DeepAgentsOptions;
    const HARNESS: Harness = Harness::DeepAgents;
    const USES_MODEL_ENDPOINT: bool = false;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::ToolApproval,
                Capability::ToolFiltering,
                Capability::History,
                Capability::CustomTools,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([
                PermissionMode::ReadOnly,
                PermissionMode::Ask,
                PermissionMode::Edit,
                PermissionMode::Full,
            ]),
        }
    }

    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        self.get_options(context)?;
        require_model(context)?;
        Ok(())
    }
}

#[must_use]
pub fn recursion_limit<S>(context: &SessionContext<S>) -> u32 {
    let option_limit = context
        .settings
        .options
        .as_ref()
        .and_then(DeepAgentsOptions::pick)
        .and_then(|options| options.recursion_limit);
    if let Some(limit) = option_limit {
        return limit;
    }
    context
        .settings
        .max_turns
        .map_or(DEFAULT_RECURSION_LIMIT, |turns| {
            BASE_RECURSION_LIMIT.saturating_add(turns.saturating_mul(DEEPAGENTS_STEPS_PER_TURN))
        })
}
