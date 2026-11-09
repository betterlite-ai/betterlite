use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::dynamic_callbacks::filters::get_callback_name;
use betterlite_logging::dynamic_callbacks::filters::get_combined_callback_list;
use betterlite_logging::dynamic_callbacks::filters::is_internal_callback;
use betterlite_logging::dynamic_callbacks::filters::remove_internal_callbacks;
use betterlite_logging::dynamic_callbacks::filters::remove_logger_callbacks;
use betterlite_logging::dynamic_callbacks::params::standard_callback_params;
use betterlite_logging::dynamic_callbacks::resolve::resolve_callback_string;
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_logging::logging_privacy::request::PrivacyRequest;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod support;
use support::Lists;
use support::Markers;
use support::Outcome;
use support::Recorder;
use support::entries;
use support::entry_values;
use support::helper_error;
use support::lists_value;
use support::trusted;

pub(crate) struct Op;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    AddProcess,
    Params,
    Combined,
    Filter,
    Resolve,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(crate) struct Input {
    action: Action,
    lists: Lists,
    add: Vec<support::EntrySpec>,
    process: bool,
    resolver: BTreeMap<String, Outcome>,
    trusted: BTreeMap<String, String>,
    names: Vec<String>,
    dynamic: Option<Vec<support::EntrySpec>>,
    global: Vec<support::EntrySpec>,
    callbacks: Vec<support::EntrySpec>,
    known: Vec<String>,
    request: Value,
    inherited: bool,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.action {
            Action::AddProcess => add_process(input),
            Action::Params => params(input),
            Action::Combined => Ok(combined(input)),
            Action::Filter => Ok(filter(input)),
            Action::Resolve => Ok(resolve(input)),
        }
    }
}

fn add_process(input: &Input) -> HelperResult {
    let markers = Markers::default();
    let mut lists = input.lists.build(&markers);
    for spec in &input.add {
        lists.add_dynamic_callback(&spec.callback(&markers));
    }
    let recorder = Recorder {
        outcomes: &input.resolver,
        markers: &markers,
        calls: Mutex::new(Vec::new()),
    };
    if input.process {
        let known = |name: &str| input.known.iter().any(|candidate| candidate == name);
        lists
            .process_dynamic_callbacks(&recorder, &trusted(&input.trusted), &known)
            .map_err(|error| helper_error("DynamicCallbackError", error.to_string()))?;
    }
    Ok(json!({"lists": lists_value(&lists), "calls": recorder.recorded()}))
}

fn params(input: &Input) -> HelperResult {
    let request: PrivacyRequest = serde_json::from_value(input.request.clone())
        .map_err(|error| helper_error("InvalidInput", error.to_string()))?;
    let params = standard_callback_params(
        &request,
        &trusted(&input.trusted),
        PrivacyContext {
            message_logging_disabled: input.inherited,
        },
    )
    .map_err(|error| helper_error("DynamicCallbackError", error.to_string()))?;
    serde_json::to_value(params).map_err(|error| helper_error("InvalidInput", error.to_string()))
}

fn combined(input: &Input) -> Value {
    let markers = Markers::default();
    let dynamic = input
        .dynamic
        .as_deref()
        .map(|specs| entries(specs, &markers));
    let global = entries(&input.global, &markers);
    json!({
        "with_dynamic": entry_values(&get_combined_callback_list(dynamic.as_deref(), &global)),
        "without_dynamic": entry_values(&get_combined_callback_list(None, &global)),
    })
}

fn filter(input: &Input) -> Value {
    let callbacks = entries(&input.callbacks, &Markers::default());
    let mut names = Vec::new();
    let mut internal = Vec::new();
    for callback in &callbacks {
        names.push(get_callback_name(callback));
        internal.push(is_internal_callback(callback));
    }
    let known = |name: &str| input.known.iter().any(|candidate| candidate == name);
    json!({
        "names": names,
        "internal": internal,
        "litellm_kept": entry_values(&remove_internal_callbacks(&callbacks)),
        "logger_kept": entry_values(&remove_logger_callbacks(&callbacks, &known)),
    })
}

fn resolve(input: &Input) -> Value {
    let markers = Markers::default();
    let recorder = Recorder {
        outcomes: &input.resolver,
        markers: &markers,
        calls: Mutex::new(Vec::new()),
    };
    let team = trusted(&input.trusted);
    let mut results = Vec::new();
    for name in &input.names {
        match resolve_callback_string(name, &team, &recorder) {
            Ok(loggers) => results.push(json!({"name": name, "loggers": logger_values(&loggers)})),
            Err(_) => results.push(json!({"name": name, "error": true})),
        }
    }
    json!({"calls": recorder.recorded(), "results": results})
}

fn logger_values(loggers: &[Arc<dyn CallbackLogger>]) -> Vec<Value> {
    let first = loggers.first();
    let mut values = Vec::new();
    for logger in loggers {
        values.push(json!({
            "name": logger.callback_name(),
            "same_as_first": first.is_some_and(|first_logger| Arc::ptr_eq(first_logger, logger)),
        }));
    }
    values
}
