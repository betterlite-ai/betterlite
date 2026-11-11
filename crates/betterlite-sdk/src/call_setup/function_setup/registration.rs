use std::collections::BTreeMap;
use std::mem;

use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::dynamic_callbacks::MAX_CALLBACKS_PER_LIST;
use betterlite_logging::dynamic_callbacks::error::DynamicCallbackError;
use betterlite_logging::logging_state::types::DynamicCallbacks;
use betterlite_models::decisions::Json;

use super::CallSetupError;
use crate::call_setup::RequestMap;
use crate::call_setup::callbacks::{
    CallbackEvent, CallbackList, CallbackLists, CustomLoggers, add_known_logger_callback,
    is_same_registered_logger,
};

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
    let callbacks = take_list(request, "callbacks")?;
    let (success, async_success) = take_success_callback(request)?;
    Ok(DynamicInputs {
        callbacks,
        success,
        async_success,
        failure: take_list(request, "failure_callback")?,
    })
}

fn take_list(
    request: &mut RequestMap,
    field: &'static str,
) -> Result<Option<Vec<String>>, CallSetupError> {
    let Some(value) = request.shift_remove(field) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    string_list(value, field).map(Some)
}

type SuccessSplit = (Option<Vec<String>>, Option<Vec<String>>);

fn take_success_callback(request: &mut RequestMap) -> Result<SuccessSplit, CallSetupError> {
    let Some(listed) = take_list(request, "success_callback")? else {
        return Ok((None, None));
    };
    let mut synchronous = Vec::new();
    let mut asynchronous = Vec::new();
    for name in listed {
        if matches!(name.as_str(), "dynamodb" | "s3") {
            asynchronous.push(name);
        } else {
            synchronous.push(name);
        }
    }
    let async_names = (!asynchronous.is_empty()).then_some(asynchronous);
    Ok((Some(synchronous), async_names))
}

fn string_list(value: Json, field: &'static str) -> Result<Vec<String>, CallSetupError> {
    let Json::List(values) = value else {
        return Err(CallSetupError::FieldType {
            field,
            expected: "a list of strings",
        });
    };
    if values.len() > MAX_CALLBACKS_PER_LIST {
        return Err(CallSetupError::Callback(DynamicCallbackError::TooMany {
            list: field,
            count: values.len(),
            limit: MAX_CALLBACKS_PER_LIST,
        }));
    }
    let mut names = Vec::with_capacity(values.len());
    for item in values {
        let Json::Text(name) = item else {
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
        for list in [
            CallbackList::Input,
            CallbackList::Success,
            CallbackList::Failure,
            CallbackList::AsyncSuccess,
            CallbackList::AsyncFailure,
        ] {
            lists.add(list, &entry);
        }
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
    resolve_event(
        CallbackEvent::Success,
        mem::take(&mut lists.success),
        lists,
        loggers,
    );
    resolve_event(
        CallbackEvent::Failure,
        mem::take(&mut lists.failure),
        lists,
        loggers,
    );
}

fn resolve_event(
    event: CallbackEvent,
    entries: Vec<DynamicCallback>,
    lists: &mut CallbackLists,
    loggers: &CustomLoggers<'_>,
) {
    let mut kept = Vec::new();
    let mut known = Vec::new();
    for entry in entries {
        match &entry {
            DynamicCallback::Name(name)
                if event == CallbackEvent::Success
                    && matches!(name.as_str(), "dynamodb" | "openmeter") =>
            {
                lists.add(CallbackList::AsyncSuccess, &entry);
            }
            DynamicCallback::Name(name) if (loggers.known)(name) => {
                known.push(name.clone());
                kept.push(entry);
            }
            DynamicCallback::Name(_) | DynamicCallback::Logger(_) => kept.push(entry),
        }
    }
    match event {
        CallbackEvent::Success => lists.success = kept,
        CallbackEvent::Failure => lists.failure = kept,
    }
    for name in known {
        add_known_logger_callback(&name, event, loggers, lists);
    }
}
