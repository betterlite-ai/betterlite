use super::error::RegistryError;
use super::sink::CallbackSink;
use crate::callback_config::error::ValidationError;
use crate::callback_config::family::{conflicting_span_scope_error, cross_entry_family_error};
use crate::callback_config::options::callback_config_error;
use crate::callback_config::{CallbackVars, FeatureFlags};

pub fn validate_callback_vars(
    callback_name: Option<&str>,
    vars: &CallbackVars,
    stored: &[CallbackVars],
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    callback_config_error(callback_name, vars, flags)?;
    cross_entry_family_error(vars, stored)?;
    conflicting_span_scope_error(vars, stored)
}

// Every LiteLLM name is unavailable until M5 compiles its exporter in; none resolves to a sink
pub fn adapter_for(name: &str) -> Result<CallbackSink, RegistryError> {
    Err(RegistryError::UnavailableAdapter {
        name: name.to_owned(),
    })
}
