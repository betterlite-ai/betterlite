use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::dynamic_callbacks::MAX_CALLBACKS_PER_LIST;

use super::hooks::DeploymentHook;

// LiteLLM keeps these five lists as module globals; betterlite passes the caller's copy through every call
#[derive(Debug, Default)]
pub struct CallbackLists {
    pub input: Vec<DynamicCallback>,
    pub success: Vec<DynamicCallback>,
    pub async_success: Vec<DynamicCallback>,
    pub failure: Vec<DynamicCallback>,
    pub async_failure: Vec<DynamicCallback>,
}

// The catalog of custom logger names and the resolver that builds them, both injected by the caller
pub struct CustomLoggers<'call> {
    pub known: &'call dyn Fn(&str) -> bool,
    pub resolver: &'call dyn LoggerResolver,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackEvent {
    Success,
    Failure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackList {
    Input,
    Success,
    AsyncSuccess,
    Failure,
    AsyncFailure,
}

impl CallbackLists {
    pub fn add(&mut self, list: CallbackList, callback: &DynamicCallback) {
        // The manager routes dynamodb and openmeter to the async list even when the caller named them for success
        if list == CallbackList::Success
            && matches!(callback, DynamicCallback::Name(name) if matches!(name.as_str(), "dynamodb" | "openmeter"))
        {
            return self.add(CallbackList::AsyncSuccess, callback);
        }
        let target = match list {
            CallbackList::Input => &mut self.input,
            CallbackList::Success => &mut self.success,
            CallbackList::AsyncSuccess => &mut self.async_success,
            CallbackList::Failure => &mut self.failure,
            CallbackList::AsyncFailure => &mut self.async_failure,
        };
        if target.iter().any(|existing| {
            existing == callback
                || (list != CallbackList::Input && is_same_registered_logger(existing, callback))
        }) {
            return;
        }
        if target.len() >= MAX_CALLBACKS_PER_LIST {
            tracing::warn!(
                limit = MAX_CALLBACKS_PER_LIST,
                "the callback list is full, the callback is dropped"
            );
            return;
        }
        target.push(callback.clone());
    }
}

// LiteLLM compares the exact class and the callback name; betterlite's loggers carry only the name
#[must_use]
pub fn is_same_registered_logger(existing: &DynamicCallback, candidate: &DynamicCallback) -> bool {
    match (existing, candidate) {
        (DynamicCallback::Logger(left), DynamicCallback::Logger(right)) => {
            left.callback_name() == right.callback_name()
        }
        (DynamicCallback::Name(_), _) | (_, DynamicCallback::Name(_)) => false,
    }
}

pub fn add_known_logger_callback(
    callback: &str,
    event: CallbackEvent,
    loggers: &CustomLoggers<'_>,
    lists: &mut CallbackLists,
) {
    if !(loggers.known)(callback) {
        tracing::debug!(callback, "the callback is not a known custom logger");
        return;
    }
    let Ok(Some(logger)) = loggers.resolver.resolve(callback, &BTreeMap::new()) else {
        return;
    };
    let entry = DynamicCallback::Logger(logger);
    let mut existing = match event {
        CallbackEvent::Success => lists.success.iter().chain(&lists.async_success),
        CallbackEvent::Failure => lists.failure.iter().chain(&lists.async_failure),
    };
    if existing.any(|found| is_same_registered_logger(found, &entry)) {
        return;
    }
    match event {
        CallbackEvent::Success => {
            lists.add(CallbackList::Success, &entry);
            lists.add(CallbackList::AsyncSuccess, &entry);
            remove_name(&mut lists.success, callback);
            remove_name(&mut lists.async_success, callback);
        }
        CallbackEvent::Failure => {
            lists.add(CallbackList::Failure, &entry);
            lists.add(CallbackList::AsyncFailure, &entry);
            remove_name(&mut lists.failure, callback);
            remove_name(&mut lists.async_failure, callback);
        }
    }
}

fn remove_name(list: &mut Vec<DynamicCallback>, name: &str) {
    let mut removed = false;
    list.retain(|entry| {
        if !removed && matches!(entry, DynamicCallback::Name(candidate) if candidate == name) {
            removed = true;
            return false;
        }
        true
    });
}

#[must_use]
pub fn get_dynamic_callbacks(
    global: &[Arc<dyn DeploymentHook>],
    dynamic: Option<&[String]>,
) -> Vec<DynamicCallback> {
    let mut callbacks: Vec<DynamicCallback> = global
        .iter()
        .map(|hook| {
            let shared = Arc::clone(hook);
            let logger: Arc<dyn CallbackLogger> = shared;
            DynamicCallback::Logger(logger)
        })
        .collect();
    for name in dynamic.unwrap_or_default() {
        callbacks.push(DynamicCallback::Name(name.clone()));
    }
    callbacks
}
