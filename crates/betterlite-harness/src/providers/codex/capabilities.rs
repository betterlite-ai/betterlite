use std::collections::BTreeSet;

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::options::CodexOptions;
use crate::transformation::BaseHarnessConfig;
use crate::types::{Capabilities, Capability, Harness, PermissionMode};

use super::CodexHarnessConfig;
use super::config::config_overrides;

impl BaseHarnessConfig for CodexHarnessConfig {
    type Options = CodexOptions;
    const HARNESS: Harness = Harness::Codex;

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supported: BTreeSet::from([
                Capability::StructuredOutput,
                Capability::Skills,
                Capability::Resume,
            ]),
            permission_modes: BTreeSet::from([PermissionMode::ReadOnly, PermissionMode::Full]),
        }
    }

    fn validate_environment<S>(&self, context: &SessionContext<S>) -> Result<(), HarnessError> {
        let options = self.get_options(context)?;
        config_overrides(&options.config).map(|_| ())
    }
}
