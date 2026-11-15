use betterlite_config::sections::guardrail_identity::item::CallbackArgs;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use betterlite_logging::registry::CallbackRegistry;
use betterlite_logging::registry::phases::CallbackList;

use super::error::StartupError;
use super::legacy::LegacyCallbackStarter;

pub struct RegistryStarter<'registry> {
    pub registry: &'registry mut CallbackRegistry,
    pub flags: FeatureFlags,
}

impl LegacyCallbackStarter for RegistryStarter<'_> {
    fn is_started(&self, callback: &str) -> bool {
        self.registry
            .list_identities(CallbackList::Callbacks)
            .iter()
            .any(|identity| identity.class_name == callback)
    }

    // the registry takes no per-callback args, so callback_args stays in the plan for the guardrail callbacks
    fn start(
        &mut self,
        callbacks: &[String],
        _callback_args: &CallbackArgs,
    ) -> Result<(), StartupError> {
        for name in callbacks {
            // AlreadyRegistered and SkippedCapacity are not errors, the registry logs them
            self.registry
                .register_configured(
                    name,
                    &CallbackVars::new(),
                    CallbackList::Callbacks,
                    self.flags,
                )
                .map_err(|source| StartupError::Legacy {
                    message: source.to_string(),
                })?;
        }
        Ok(())
    }
}
