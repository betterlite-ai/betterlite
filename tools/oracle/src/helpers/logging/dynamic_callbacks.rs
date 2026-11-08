use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::dynamic_callbacks::DynamicCallbackLists;
use betterlite_logging::dynamic_callbacks::error::DynamicCallbackError;
use betterlite_logging::dynamic_callbacks::filters::get_callback_name;
use betterlite_logging::dynamic_callbacks::filters::get_combined_callback_list;
use betterlite_logging::dynamic_callbacks::filters::is_internal_callback;
use betterlite_logging::dynamic_callbacks::filters::remove_internal_callbacks;
use betterlite_logging::dynamic_callbacks::filters::remove_logger_callbacks;
use betterlite_logging::dynamic_callbacks::params::standard_callback_params;
use betterlite_logging::dynamic_callbacks::resolve::resolve_callback_string;
use betterlite_logging::logging_privacy::PrivacyContext;
use betterlite_logging::logging_privacy::callbacks::TrustedCallbacks;
use betterlite_logging::logging_privacy::request::PrivacyRequest;
use betterlite_logging::registry::error::RegistryError;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};

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
    add: Vec<EntrySpec>,
    process: bool,
    resolver: BTreeMap<String, Outcome>,
    trusted: BTreeMap<String, String>,
    names: Vec<String>,
    dynamic: Option<Vec<EntrySpec>>,
    global: Vec<EntrySpec>,
    callbacks: Vec<EntrySpec>,
    request: Value,
    inherited: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Lists {
    input: Option<Vec<EntrySpec>>,
    success: Option<Vec<EntrySpec>>,
    async_success: Option<Vec<EntrySpec>>,
    failure: Option<Vec<EntrySpec>>,
    async_failure: Option<Vec<EntrySpec>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EntrySpec {
    Name(String),
    Logger { logger: String },
}

impl EntrySpec {
    fn callback(&self, markers: &Markers) -> DynamicCallback {
        match self {
            Self::Name(name) => DynamicCallback::Name(name.clone()),
            Self::Logger { logger } => DynamicCallback::Logger(markers.get(logger)),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct Outcome {
    label: Option<String>,
    empty_label: Option<String>,
    error: bool,
}

struct MarkerLogger {
    label: String,
}

#[async_trait]
impl CallbackLogger for MarkerLogger {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

#[derive(Default)]
struct Markers {
    labels: Mutex<BTreeMap<String, Arc<MarkerLogger>>>,
}

impl Markers {
    fn get(&self, label: &str) -> Arc<MarkerLogger> {
        let mut labels = self.labels.lock().unwrap_or_else(PoisonError::into_inner);
        let logger = labels.entry(label.to_owned()).or_insert_with(|| {
            Arc::new(MarkerLogger {
                label: label.to_owned(),
            })
        });
        let cloned = Arc::clone(logger);
        drop(labels);
        cloned
    }
}

#[derive(Serialize)]
struct Recorded {
    name: String,
    params: BTreeMap<String, String>,
}

struct Recorder<'input> {
    outcomes: &'input BTreeMap<String, Outcome>,
    markers: &'input Markers,
    calls: Mutex<Vec<Recorded>>,
}

impl Recorder<'_> {
    fn recorded(&self) -> Vec<Recorded> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls
            .iter()
            .map(|call| Recorded {
                name: call.name.clone(),
                params: call.params.clone(),
            })
            .collect()
    }
}

impl LoggerResolver for Recorder<'_> {
    fn resolve(
        &self,
        name: &str,
        params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                name: name.to_owned(),
                params: params.clone(),
            });
        let Some(outcome) = self.outcomes.get(name) else {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        };
        if outcome.error {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        }
        let selected = if params.is_empty() {
            outcome.empty_label.as_ref().or(outcome.label.as_ref())
        } else {
            outcome.label.as_ref()
        };
        let Some(label) = selected else {
            return Ok(None);
        };
        let logger: Arc<dyn CallbackLogger> = self.markers.get(label);
        Ok(Some(logger))
    }
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
        lists
            .process_dynamic_callbacks(&recorder, &trusted(&input.trusted))
            .map_err(|error| failure(&error))?;
    }
    Ok(json!({"lists": lists_value(&lists), "calls": recorder.recorded()}))
}

fn params(input: &Input) -> HelperResult {
    let request: PrivacyRequest = serde_json::from_value(input.request.clone())
        .map_err(|error| invalid_input(&error.to_string()))?;
    let params = standard_callback_params(
        &request,
        &trusted(&input.trusted),
        PrivacyContext {
            message_logging_disabled: input.inherited,
        },
    )
    .map_err(|error| failure(&error))?;
    serde_json::to_value(params).map_err(|error| invalid_input(&error.to_string()))
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
    json!({
        "names": callbacks.iter().map(get_callback_name).collect::<Vec<_>>(),
        "internal": callbacks.iter().map(is_internal_callback).collect::<Vec<_>>(),
        "litellm_kept": entry_values(&remove_internal_callbacks(&callbacks)),
        "logger_kept": entry_values(&remove_logger_callbacks(&callbacks)),
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
            Ok(loggers) => {
                let first = loggers.first();
                let values: Vec<Value> = loggers
                    .iter()
                    .map(|logger| {
                        json!({
                            "name": logger.callback_name(),
                            "same_as_first": first.is_some_and(|first_logger| {
                                Arc::ptr_eq(first_logger, logger)
                            }),
                        })
                    })
                    .collect();
                results.push(json!({"name": name, "loggers": values}));
            }
            Err(_) => results.push(json!({"name": name, "error": true})),
        }
    }
    json!({"calls": recorder.recorded(), "results": results})
}

impl Lists {
    fn build(&self, markers: &Markers) -> DynamicCallbackLists {
        DynamicCallbackLists {
            input: self.input.as_deref().map(|specs| entries(specs, markers)),
            success: self.success.as_deref().map(|specs| entries(specs, markers)),
            async_success: self
                .async_success
                .as_deref()
                .map(|specs| entries(specs, markers)),
            failure: self.failure.as_deref().map(|specs| entries(specs, markers)),
            async_failure: self
                .async_failure
                .as_deref()
                .map(|specs| entries(specs, markers)),
        }
    }
}

fn entries(specs: &[EntrySpec], markers: &Markers) -> Vec<DynamicCallback> {
    let mut entries = Vec::new();
    for spec in specs {
        entries.push(spec.callback(markers));
    }
    entries
}

fn lists_value(lists: &DynamicCallbackLists) -> Value {
    json!({
        "input": list_value(lists.input.as_deref()),
        "success": list_value(lists.success.as_deref()),
        "async_success": list_value(lists.async_success.as_deref()),
        "failure": list_value(lists.failure.as_deref()),
        "async_failure": list_value(lists.async_failure.as_deref()),
    })
}

fn list_value(entries: Option<&[DynamicCallback]>) -> Value {
    let Some(values) = entries else {
        return Value::Null;
    };
    json!(entry_values(values))
}

fn entry_values(entries: &[DynamicCallback]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| match entry {
            DynamicCallback::Name(name) => json!(name),
            DynamicCallback::Logger(logger) => json!({"logger": logger.callback_name()}),
        })
        .collect()
}

fn trusted(values: &BTreeMap<String, String>) -> TrustedCallbacks {
    TrustedCallbacks {
        values: values
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
            .collect(),
    }
}

fn failure(error: &DynamicCallbackError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "DynamicCallbackError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn invalid_input(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
