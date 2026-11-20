use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;

use crate::python_hooks::slots::{LoadedCallback, LoadedHook};

// litellm.audit_log_callbacks holds logger instances, configured names and loaded Python objects
#[derive(Clone)]
pub enum AuditCallbackEntry {
    Logger(Arc<dyn CallbackLogger>),
    Named(String),
    Python(Arc<LoadedHook>),
}

impl From<LoadedCallback> for AuditCallbackEntry {
    fn from(callback: LoadedCallback) -> Self {
        match callback {
            LoadedCallback::Named(name) => Self::Named(name),
            LoadedCallback::Loaded(hook) => Self::Python(Arc::new(hook)),
        }
    }
}
