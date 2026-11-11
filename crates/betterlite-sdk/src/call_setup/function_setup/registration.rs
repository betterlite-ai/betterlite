use std::collections::BTreeMap;
use std::mem;

use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::logging_state::types::DynamicCallbacks;
use betterlite_models::decisions::Json;

use super::CallSetupError;
use crate::call_setup::RequestMap;
use crate::call_setup::callbacks::CallbackLists;
use crate::call_setup::callbacks::CustomLoggers;
use crate::call_setup::callbacks::custom_logger::CallbackEvent;
use crate::call_setup::callbacks::custom_logger::add_known_logger_callback;
use crate::call_setup::callbacks::custom_logger::is_same_registered_logger;

#[derive(Debug)]
pub(super) struct DynamicInputs {
    pub(super) callbacks: Option<Vec<String>>,
    success: Option<Vec<String>>,
    async_success: Option<Vec<String>>,
    failure: Option<Vec<String>>,
}

impl DynamicInputs {
    pub(super) fn logging(&self) -> DynamicCallbacks {
        DynamicCallbacks {
            input: None,
            success: self.success.clone(),
            async_success: self.async_success.clone(),
            failure: self.failure.clone(),
            async_failure: None,
        }
    }
}

pub(super) fn take_dynamic_callbacks(
    request: &mut RequestMap,
) -> Result<DynamicInputs, CallSetupError> {
    let callbacks = match request.shift_remove("callbacks") {
        None | Some(Json::Null) => None,
        Some(Json::List(values)) => Some(string_list(values, "callbacks")?),
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "callbacks",
                expected: "a list of strings",
            });
        }
    };
    let (success, async_success) = take_success_callback(request)?;
    let failure = take_failure_callback(request)?;
    Ok(DynamicInputs {
        callbacks,
        success,
        async_success,
        failure,
    })
}

type SuccessSplit = (Option<Vec<String>>, Option<Vec<String>>);

fn take_success_callback(request: &mut RequestMap) -> Result<SuccessSplit, CallSetupError> {
    let listed = match request.get("success_callback").cloned() {
        None | Some(Json::Null) => return Ok((None, None)),
        Some(Json::List(values)) => values,
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "success_callback",
                expected: "a list of strings",
            });
        }
    };
    request.shift_remove("success_callback");
    let mut sync = Vec::new();
    let mut asynchronous = Vec::new();
    for value in listed {
        let Json::Text(name) = value else {
            return Err(CallSetupError::FieldType {
                field: "success_callback",
                expected: "a list of strings",
            });
        };
        if matches!(name.as_str(), "dynamodb" | "s3") {
            asynchronous.push(name);
        } else {
            sync.push(name);
        }
    }
    let async_names = (!asynchronous.is_empty()).then_some(asynchronous);
    Ok((Some(sync), async_names))
}

fn take_failure_callback(request: &mut RequestMap) -> Result<Option<Vec<String>>, CallSetupError> {
    let listed = match request.get("failure_callback").cloned() {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::List(values)) => values,
        Some(_) => {
            return Err(CallSetupError::FieldType {
                field: "failure_callback",
                expected: "a list of strings",
            });
        }
    };
    request.shift_remove("failure_callback");
    Ok(Some(string_list(listed, "failure_callback")?))
}

fn string_list(values: Vec<Json>, field: &'static str) -> Result<Vec<String>, CallSetupError> {
    let mut names = Vec::with_capacity(values.len());
    for value in values {
        let Json::Text(name) = value else {
            return Err(CallSetupError::FieldType {
                field,
                expected: "a list of strings",
            });
        };
        names.push(name);
    }
    Ok(names)
}

pub(super) fn register_request_callbacks(
    callbacks: &[DynamicCallback],
    loggers: &CustomLoggers<'_>,
    lists: &mut CallbackLists,
) {
    for callback in callbacks {
        let Some(entry) = resolve_request_callback(callback, loggers, lists) else {
            continue;
        };
        lists.add_input(&entry);
        lists.add_success(&entry);
        lists.add_failure(&entry);
        lists.add_async_success(&entry);
        lists.add_async_failure(&entry);
    }
}

fn resolve_request_callback(
    callback: &DynamicCallback,
    loggers: &CustomLoggers<'_>,
    lists: &CallbackLists,
) -> Option<DynamicCallback> {
    let DynamicCallback::Name(name) = callback else {
        return Some(callback.clone());
    };
    let Ok(Some(logger)) = loggers.resolver.resolve(name, &BTreeMap::new()) else {
        return None;
    };
    let entry = DynamicCallback::Logger(logger);
    if lists
        .async_success
        .iter()
        .any(|existing| is_same_registered_logger(existing, &entry))
    {
        return None;
    }
    Some(entry)
}

// LiteLLM removes each resolved name from the list it is scanning, so only the last pass touches every name
pub(super) fn resolve_global_callbacks(lists: &mut CallbackLists, loggers: &CustomLoggers<'_>) {
    for (event, originals) in [
        (CallbackEvent::Success, mem::take(&mut lists.success)),
        (CallbackEvent::Failure, mem::take(&mut lists.failure)),
    ] {
        let mut known = Vec::new();
        for entry in originals {
            match &entry {
                DynamicCallback::Name(name)
                    if event == CallbackEvent::Success
                        && matches!(name.as_str(), "dynamodb" | "openmeter") =>
                {
                    lists.add_async_success(&entry);
                }
                DynamicCallback::Name(name) if (loggers.known)(name) => {
                    known.push(name.clone());
                    push_event_list(event, lists, entry);
                }
                DynamicCallback::Name(_) | DynamicCallback::Logger(_) => {
                    push_event_list(event, lists, entry);
                }
            }
        }
        for name in known {
            add_known_logger_callback(&name, event, loggers, lists);
        }
    }
}

fn push_event_list(event: CallbackEvent, lists: &mut CallbackLists, entry: DynamicCallback) {
    match event {
        CallbackEvent::Success => lists.success.push(entry),
        CallbackEvent::Failure => lists.failure.push(entry),
    }
}
