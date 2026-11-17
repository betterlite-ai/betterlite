use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::otel::v2_callback::callback::V2Callback;
use crate::registry::phases::CallbackList;

// the four proxy lists LiteLLM registers the callback in, named by the server's own callback registry
pub const CALLBACK_LISTS: [CallbackList; 4] = [
    CallbackList::Service,
    CallbackList::Input,
    CallbackList::Success,
    CallbackList::Failure,
];

const OTEL_MODULE_PREFIXES: [&str; 2] = [
    "litellm.integrations.otel",
    "litellm.integrations.opentelemetry",
];

#[derive(Clone)]
pub enum CallbackEntry {
    OpenTelemetry(Arc<V2Callback>),
    Named(String),
}

impl CallbackEntry {
    // litellm.integrations.otel.logger._register_in_callback_list's module-prefix check
    fn is_open_telemetry(&self) -> bool {
        match self {
            Self::OpenTelemetry(_) => true,
            Self::Named(module) => OTEL_MODULE_PREFIXES
                .iter()
                .any(|prefix| module.starts_with(*prefix)),
        }
    }
}

// the proxy's callback lists and open_telemetry_logger slot, injected rather than module globals (docs/deviations/B3853.yaml)
#[derive(Default)]
pub struct V2Registry {
    lists: Mutex<BTreeMap<CallbackList, Vec<CallbackEntry>>>,
    open_telemetry_logger: Mutex<Option<Arc<V2Callback>>>,
}

impl V2Registry {
    pub fn register(&self, callback: &Arc<V2Callback>) {
        for list in CALLBACK_LISTS {
            self.register_once(list, callback);
        }
        let mut slot = self.slot();
        if slot.is_none() {
            *slot = Some(Arc::clone(callback));
        }
    }

    // another callback the proxy already had; a named OTel-module entry blocks this registration too
    pub fn add_named_callback(&self, module: &str) {
        for list in CALLBACK_LISTS {
            self.lists()
                .entry(list)
                .or_default()
                .push(CallbackEntry::Named(module.to_owned()));
        }
    }

    #[must_use]
    pub fn entries(&self, list: CallbackList) -> Vec<CallbackEntry> {
        match self.lists().get(&list) {
            Some(entries) => entries.clone(),
            None => Vec::new(),
        }
    }

    // the first logger to register keeps the slot, so later instances are not reachable through it
    #[must_use]
    pub fn registered_v2_logger(&self) -> Option<Arc<V2Callback>> {
        match self.open_telemetry_logger.lock() {
            Ok(slot) => slot.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn register_once(&self, list: CallbackList, callback: &Arc<V2Callback>) {
        let mut lists = self.lists();
        let entries = lists.entry(list).or_default();
        if !entries.iter().any(CallbackEntry::is_open_telemetry) {
            entries.push(CallbackEntry::OpenTelemetry(Arc::clone(callback)));
        }
    }

    fn lists(&self) -> MutexGuard<'_, BTreeMap<CallbackList, Vec<CallbackEntry>>> {
        match self.lists.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<Arc<V2Callback>>> {
        match self.open_telemetry_logger.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
