use std::collections::BTreeMap;

use betterlite_logging::dynamic_callbacks::DynamicCallback;

use super::{CallbackLists, CustomLoggers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackEvent {
    Success,
    Failure,
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
    match event {
        CallbackEvent::Success => {
            if !registered_success(lists, &entry) {
                lists.add_success(&entry);
                lists.add_async_success(&entry);
                remove_name(&mut lists.success, callback);
                remove_name(&mut lists.async_success, callback);
            }
        }
        CallbackEvent::Failure => {
            if !registered_failure(lists, &entry) {
                lists.add_failure(&entry);
                lists.add_async_failure(&entry);
                remove_name(&mut lists.failure, callback);
                remove_name(&mut lists.async_failure, callback);
            }
        }
    }
}

fn registered_success(lists: &CallbackLists, entry: &DynamicCallback) -> bool {
    lists
        .success
        .iter()
        .chain(lists.async_success.iter())
        .any(|existing| is_same_registered_logger(existing, entry))
}

fn registered_failure(lists: &CallbackLists, entry: &DynamicCallback) -> bool {
    lists
        .failure
        .iter()
        .chain(lists.async_failure.iter())
        .any(|existing| is_same_registered_logger(existing, entry))
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
