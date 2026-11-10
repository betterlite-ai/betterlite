use std::sync::Arc;

use crate::callback_logger::CallbackLogger;
use crate::dynamic_callbacks::{DynamicCallback, DynamicCallbackLists};
use crate::registry::CallbackRegistry;
use crate::registry::phases::CallbackPhase;

pub struct FailureCallbacks<'callbacks> {
    pub registry: &'callbacks CallbackRegistry,
    pub dynamic: Option<&'callbacks DynamicCallbackLists>,
}

impl FailureCallbacks<'_> {
    #[must_use]
    pub fn sync_loggers(&self) -> Vec<Arc<dyn CallbackLogger>> {
        self.dynamic.map_or_else(
            || self.combined(None),
            |lists| self.combined(lists.failure.as_deref()),
        )
    }

    #[must_use]
    pub fn async_loggers(&self) -> Vec<Arc<dyn CallbackLogger>> {
        self.dynamic.map_or_else(
            || self.combined(None),
            |lists| self.combined(lists.async_failure.as_deref()),
        )
    }

    fn combined(&self, dynamic: Option<&[DynamicCallback]>) -> Vec<Arc<dyn CallbackLogger>> {
        let mut loggers: Vec<Arc<dyn CallbackLogger>> = Vec::new();
        for callback in dynamic.unwrap_or_default() {
            let DynamicCallback::Logger(logger) = callback else {
                continue;
            };
            if !loggers.iter().any(|kept| Arc::ptr_eq(kept, logger)) {
                loggers.push(Arc::clone(logger));
            }
        }
        for logger in self.registry.loggers(CallbackPhase::Failure) {
            if !loggers.iter().any(|kept| Arc::ptr_eq(kept, &logger)) {
                loggers.push(logger);
            }
        }
        loggers
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn should_run_sync_failure_callbacks_for_async_calls(&self) -> bool {
        let legacy_dynamic = self
            .dynamic
            .and_then(|lists| lists.failure.as_deref())
            .is_some_and(|callbacks| {
                callbacks
                    .iter()
                    .any(|callback| matches!(callback, DynamicCallback::Name(_)))
            });
        let legacy_registry = self.registry.loggers(CallbackPhase::Failure).len()
            < self
                .registry
                .phase_identities(CallbackPhase::Failure)
                .count();
        legacy_dynamic || legacy_registry
    }
}
