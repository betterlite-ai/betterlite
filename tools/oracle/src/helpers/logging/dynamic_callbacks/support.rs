use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::dynamic_callbacks::DynamicCallbackLists;
use betterlite_logging::dynamic_callbacks::filters::get_callback_name;
use betterlite_logging::logging_privacy::callbacks::TrustedCallbacks;
use betterlite_logging::registry::error::RegistryError;
use secrecy::SecretString;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::compare::helper::HelperError;

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum EntrySpec {
    Name(String),
    Logger { logger: String },
}

impl EntrySpec {
    pub(super) fn callback(&self, markers: &Markers) -> DynamicCallback {
        match self {
            Self::Name(name) => DynamicCallback::Name(name.clone()),
            Self::Logger { logger } => DynamicCallback::Logger(markers.get(logger)),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct Outcome {
    label: Option<String>,
    empty_label: Option<String>,
    error: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct Lists {
    input: Option<Vec<EntrySpec>>,
    success: Option<Vec<EntrySpec>>,
    async_success: Option<Vec<EntrySpec>>,
    failure: Option<Vec<EntrySpec>>,
    async_failure: Option<Vec<EntrySpec>>,
}

impl Lists {
    pub(super) fn build(&self, markers: &Markers) -> DynamicCallbackLists {
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
pub(super) struct Markers {
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
pub(super) struct Recorded {
    name: String,
    params: BTreeMap<String, String>,
}

pub(super) struct Recorder<'input> {
    pub(super) outcomes: &'input BTreeMap<String, Outcome>,
    pub(super) markers: &'input Markers,
    pub(super) calls: Mutex<Vec<Recorded>>,
}

impl Recorder<'_> {
    pub(super) fn recorded(&self) -> Vec<Recorded> {
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

pub(super) fn entries(specs: &[EntrySpec], markers: &Markers) -> Vec<DynamicCallback> {
    let mut entries = Vec::new();
    for spec in specs {
        entries.push(spec.callback(markers));
    }
    entries
}

pub(super) fn lists_value(lists: &DynamicCallbackLists) -> Value {
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

pub(super) fn entry_values(entries: &[DynamicCallback]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| match entry {
            DynamicCallback::Name(name) => json!(name),
            DynamicCallback::Logger(_) => json!({"logger": get_callback_name(entry)}),
        })
        .collect()
}

pub(super) fn trusted(values: &BTreeMap<String, String>) -> TrustedCallbacks {
    TrustedCallbacks {
        values: values
            .iter()
            .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
            .collect(),
    }
}

pub(super) fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
