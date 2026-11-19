use std::sync::Arc;

use async_trait::async_trait;
use betterlite_logging::audit::payload::StandardAuditLogPayload;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_pyhost::hook_object::{LoadedKind, MethodCall};
use betterlite_pyhost::json_values::JsonObject;

use crate::python_hooks::deadline::HOOK_DEADLINE;
use crate::python_hooks::slots::LoadedHook;

// B6030 ships the class at this path; until then an object never matches it and stays ignored
const LOGGER_CLASS: &str = "betterlite.integrations.custom_logger.CustomLogger";
const AUDIT_PYTHON_CALLBACK: &str = "python_audit_log_callback";

pub struct PythonAuditCallback {
    pub hook: Arc<LoadedHook>,
}

// LiteLLM dispatches only a configured CustomLogger instance and silently ignores any other loaded object
#[must_use]
pub fn audit_logger(hook: Arc<LoadedHook>) -> Option<Arc<dyn CallbackLogger>> {
    match hook.object.kind(&hook.host, LOGGER_CLASS) {
        LoadedKind::Logger => Some(Arc::new(PythonAuditCallback { hook })),
        LoadedKind::Function | LoadedKind::Class { .. } | LoadedKind::Other { .. } => None,
    }
}

#[async_trait]
impl CallbackLogger for PythonAuditCallback {
    fn callback_name(&self) -> &str {
        AUDIT_PYTHON_CALLBACK
    }

    async fn async_log_audit_log_event(
        &self,
        audit_log: &StandardAuditLogPayload,
    ) -> Result<(), LoggerError> {
        let payload = serde_json::to_value(audit_log).map_err(|source| delivery_error(&source))?;
        self.hook
            .object
            .call_method(
                &self.hook.host,
                &self.hook.tracker,
                HOOK_DEADLINE,
                MethodCall {
                    name: "async_log_audit_log_event",
                    args: vec![payload],
                    kwargs: JsonObject::new(),
                },
            )
            .await
            .map_err(|source| delivery_error(&source))?;
        Ok(())
    }
}

fn delivery_error<Source>(source: &Source) -> LoggerError
where
    Source: std::fmt::Display,
{
    LoggerError::Delivery {
        callback: AUDIT_PYTHON_CALLBACK.to_owned(),
        message: source.to_string(),
    }
}
