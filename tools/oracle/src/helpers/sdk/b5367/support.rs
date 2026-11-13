use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;

use async_trait::async_trait;
use betterlite_core::rules::RuleDecision;
use betterlite_core::rules::RuleEvaluator;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;
use betterlite_logging::registry::error::RegistryError;
use betterlite_sdk::call_setup::callbacks::CallbackLists;
use betterlite_sdk::call_setup::credentials::Credential;
use betterlite_sdk::call_setup::hooks::DeploymentHook;
use betterlite_sdk::call_setup::hooks::Guardrail;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

use crate::helpers::HelperError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CredentialSpec {
    pub(super) credential_name: String,
    pub(super) credential_values: BTreeMap<String, String>,
}

impl CredentialSpec {
    pub(super) fn credential(&self) -> Credential {
        let mut credential_values = IndexMap::new();
        for (key, value) in &self.credential_values {
            credential_values.insert(key.clone(), SecretString::from(value.clone()));
        }
        Credential {
            credential_name: self.credential_name.clone(),
            credential_values,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum EntrySpec {
    Name(String),
    Logger(LoggerSpec),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LoggerSpec {
    pub(super) logger: String,
    #[serde(default)]
    pub(super) guardrail: Option<GuardrailSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GuardrailSpec {
    #[serde(default)]
    pub(super) name: Option<String>,
    pub(super) default_on: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct OutcomeSpec {
    pub(super) label: Option<String>,
    pub(super) error: bool,
}

pub(super) struct Marker {
    label: String,
    guardrail: Option<Guardrail>,
}

#[async_trait]
impl CallbackLogger for Marker {
    fn callback_name(&self) -> &str {
        &self.label
    }
}

#[async_trait]
impl DeploymentHook for Marker {
    fn guardrail(&self) -> Option<Guardrail> {
        self.guardrail.clone()
    }
}

#[derive(Default)]
pub(super) struct Markers {
    entries: Mutex<BTreeMap<String, Arc<Marker>>>,
}

impl Markers {
    pub(super) fn with_guardrail(&self, label: &str, guardrail: Option<Guardrail>) -> Arc<Marker> {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(entries.entry(label.to_owned()).or_insert_with(|| {
            Arc::new(Marker {
                label: label.to_owned(),
                guardrail,
            })
        }))
    }
}

pub(super) struct Resolver<'input> {
    pub(super) outcomes: &'input BTreeMap<String, OutcomeSpec>,
    pub(super) markers: &'input Markers,
}

impl LoggerResolver for Resolver<'_> {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        let Some(spec) = self.outcomes.get(name) else {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        };
        if spec.error {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        }
        Ok(spec.label.as_deref().map(|label| {
            let logger: Arc<dyn CallbackLogger> = self.markers.with_guardrail(label, None);
            logger
        }))
    }
}

pub(super) fn global_callback(spec: &EntrySpec, markers: &Markers) -> Arc<dyn DeploymentHook> {
    match spec {
        EntrySpec::Name(name) => {
            let hook: Arc<dyn DeploymentHook> = markers.with_guardrail(name, None);
            hook
        }
        EntrySpec::Logger(logger_spec) => {
            let guardrail = logger_spec.guardrail.as_ref().map(guardrail);
            let hook: Arc<dyn DeploymentHook> =
                markers.with_guardrail(&logger_spec.logger, guardrail);
            hook
        }
    }
}

fn guardrail(spec: &GuardrailSpec) -> Guardrail {
    Guardrail {
        name: spec.name.clone(),
        default_on: spec.default_on,
    }
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
    pub(super) fn build(&self, markers: &Markers) -> CallbackLists {
        CallbackLists {
            input: entries(self.input.as_deref(), markers),
            success: entries(self.success.as_deref(), markers),
            async_success: entries(self.async_success.as_deref(), markers),
            failure: entries(self.failure.as_deref(), markers),
            async_failure: entries(self.async_failure.as_deref(), markers),
        }
    }
}

fn entries(listed: Option<&[EntrySpec]>, markers: &Markers) -> Vec<DynamicCallback> {
    let Some(specs) = listed else {
        return Vec::new();
    };
    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        match spec {
            EntrySpec::Name(name) => entries.push(DynamicCallback::Name(name.clone())),
            EntrySpec::Logger(logger_spec) => {
                let guardrail = logger_spec.guardrail.as_ref().map(guardrail);
                let logger: Arc<dyn CallbackLogger> =
                    markers.with_guardrail(&logger_spec.logger, guardrail);
                entries.push(DynamicCallback::Logger(logger));
            }
        }
    }
    entries
}

pub(super) fn entry_value(entry: &DynamicCallback) -> Value {
    match entry {
        DynamicCallback::Name(name) => json!(name),
        DynamicCallback::Logger(logger) => json!({"logger": logger.callback_name()}),
    }
}

pub(super) fn lists_value(lists: &CallbackLists) -> Value {
    let mut value = serde_json::Map::new();
    for (key, entries) in [
        ("input", &lists.input),
        ("success", &lists.success),
        ("async_success", &lists.async_success),
        ("failure", &lists.failure),
        ("async_failure", &lists.async_failure),
    ] {
        value.insert(
            key.to_owned(),
            Value::Array(entries.iter().map(entry_value).collect()),
        );
    }
    Value::Object(value)
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Decision {
    Boolean(bool),
    Dict {
        decision: Option<bool>,
        message: Option<String>,
    },
}

pub(super) struct Rules<'rules> {
    pub(super) rules: &'rules [Option<Decision>],
}

impl RuleEvaluator for Rules<'_> {
    fn evaluate(&self, index: usize, _input: &str) -> Option<RuleDecision> {
        let rule = self.rules.get(index)?;
        Some(match rule {
            None => RuleDecision::NotCallable,
            Some(Decision::Boolean(value)) => RuleDecision::Boolean(*value),
            Some(Decision::Dict { decision, message }) => RuleDecision::Dict {
                decision: *decision,
                message: message.clone(),
            },
        })
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
