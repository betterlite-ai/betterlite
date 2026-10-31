use std::collections::BTreeSet;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::options::OpenCodeOptions;
use crate::transformation::BaseHarnessConfig;
use crate::types::{Capabilities, Capability, Harness, PermissionMode};

use super::OpenCodeHarnessConfig;
use super::config::validate_user_config;

impl BaseHarnessConfig for OpenCodeHarnessConfig {
    type Options = OpenCodeOptions;
    const HARNESS: Harness = Harness::OpenCode;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::ToolFiltering,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([
                PermissionMode::ReadOnly,
                PermissionMode::Edit,
                PermissionMode::Full,
            ]),
        }
    }

    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        let options = self.get_options(context)?;
        validate_user_config(&options.config)
    }
}
