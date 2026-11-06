use betterlite_logging::callback_config::error::ValidationError;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use betterlite_logging::registry::catalog::validate_callback_vars;
use betterlite_logging::registry::identity::{Attribute, CallbackIdentity};
use betterlite_logging::registry::phases::{CallbackList, CallbackPhase};
use betterlite_logging::registry::sink::CallbackSink;
use betterlite_logging::registry::{CallbackRegistry, Registration};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default = "default_capacity")]
    max_callbacks: usize,
    steps: Vec<Step>,
}

const fn default_capacity() -> usize {
    100
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Step {
    Register {
        list: ListName,
        callback: CallbackSpec,
    },
    RemoveFromList {
        list: ListName,
        callback: CallbackSpec,
    },
    RemoveClassFromList {
        list: ListName,
        class_name: String,
    },
    Validate {
        #[serde(default)]
        callback_name: Option<String>,
        #[serde(default)]
        vars: CallbackVars,
        #[serde(default)]
        stored: Vec<CallbackVars>,
        #[serde(default)]
        otel_v2: bool,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ListName {
    Callbacks,
    SuccessCallback,
    FailureCallback,
    InputCallback,
    ServiceCallback,
}

impl ListName {
    const fn list(&self) -> CallbackList {
        match self {
            Self::Callbacks => CallbackList::Callbacks,
            Self::SuccessCallback => CallbackList::Success,
            Self::FailureCallback => CallbackList::Failure,
            Self::InputCallback => CallbackList::Input,
            Self::ServiceCallback => CallbackList::Service,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallbackSpec {
    class_name: String,
    #[serde(default)]
    attributes: IndexMap<String, AttributeSpec>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AttributeSpec {
    Flag(bool),
    Integer(i64),
    Text(String),
}

impl AttributeSpec {
    fn attribute_value(&self) -> Attribute {
        match self {
            Self::Flag(set) => Attribute::Flag(*set),
            Self::Integer(number) => Attribute::Integer(*number),
            Self::Text(text) => Attribute::Text(text.clone()),
        }
    }
}

impl CallbackSpec {
    fn identity(&self) -> CallbackIdentity {
        CallbackIdentity {
            class_name: self.class_name.clone(),
            attributes: self
                .attributes
                .iter()
                .map(|(name, spec)| (name.clone(), spec.attribute_value()))
                .collect(),
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut registry = CallbackRegistry::new(input.max_callbacks);
        let steps: Vec<Value> = input
            .steps
            .iter()
            .map(|step| run_step(&mut registry, step))
            .collect();
        Ok(json!({
            "steps": steps,
            "lists": lists(&registry),
            "phases": phases(&registry),
            "selection": selection(&registry),
        }))
    }
}

fn run_step(registry: &mut CallbackRegistry, step: &Step) -> Value {
    match step {
        Step::Register { list, callback } => registration_outcome(registry.register(
            callback.identity(),
            CallbackSink::Tracing,
            list.list(),
        )),
        Step::RemoveFromList { list, callback } => {
            let removed = registry.remove_from_list(&callback.identity(), list.list());
            json!({"outcome": if removed { "removed" } else { "absent" }})
        }
        Step::RemoveClassFromList { list, class_name } => {
            let count = registry.remove_class_from_list(class_name, list.list());
            json!({"outcome": "removed", "count": count})
        }
        Step::Validate {
            callback_name,
            vars,
            stored,
            otel_v2,
        } => validation_outcome(
            callback_name.as_deref(),
            vars,
            stored,
            FeatureFlags {
                otel_v2_enabled: *otel_v2,
            },
        ),
    }
}

fn registration_outcome(outcome: Registration) -> Value {
    match outcome {
        Registration::Registered => json!({"outcome": "registered"}),
        Registration::AlreadyRegistered => json!({"outcome": "already_registered"}),
        Registration::SkippedCapacity => json!({"outcome": "skipped_capacity"}),
    }
}

fn validation_outcome(
    callback_name: Option<&str>,
    vars: &CallbackVars,
    stored: &[CallbackVars],
    flags: FeatureFlags,
) -> Value {
    match validate_callback_vars(callback_name, vars, stored, flags) {
        Ok(()) => json!({"outcome": "ok"}),
        Err(error) => json!({"outcome": "invalid", "validator": validator_of(&error)}),
    }
}

const fn validator_of(error: &ValidationError) -> &'static str {
    match error {
        ValidationError::FamilyConflict { .. } => "cross_entry_family_error",
        ValidationError::SpanScopeConflict { .. } => "conflicting_span_scope_error",
        ValidationError::SamplingRateCallback { .. }
        | ValidationError::SamplingRateUnparsable { .. }
        | ValidationError::SamplingRateOutOfRange { .. }
        | ValidationError::LangfuseEnvironment { .. }
        | ValidationError::SpanScopeCallback { .. }
        | ValidationError::SpanScopeValue { .. }
        | ValidationError::NewRelicRequiresOtelV2
        | ValidationError::NewRelicRegion { .. }
        | ValidationError::NewRelicRegionNeedsKey => "callback_config_error",
    }
}

fn keys_for_list(registry: &CallbackRegistry, list: CallbackList) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for identity in registry.list_identities(list) {
        keys.push(identity.key());
    }
    keys
}

fn keys_for_phase(registry: &CallbackRegistry, phase: CallbackPhase) -> Vec<String> {
    let mut keys: Vec<String> = registry
        .phase_identities(phase)
        .map(CallbackIdentity::key)
        .collect();
    keys.sort();
    keys
}

fn lists(registry: &CallbackRegistry) -> Value {
    json!({
        "callbacks": keys_for_list(registry, CallbackList::Callbacks),
        "success_callback": keys_for_list(registry, CallbackList::Success),
        "failure_callback": keys_for_list(registry, CallbackList::Failure),
        "input_callback": keys_for_list(registry, CallbackList::Input),
        "service_callback": keys_for_list(registry, CallbackList::Service),
    })
}

fn phases(registry: &CallbackRegistry) -> Value {
    json!({
        "input": keys_for_phase(registry, CallbackPhase::Input),
        "success": keys_for_phase(registry, CallbackPhase::Success),
        "failure": keys_for_phase(registry, CallbackPhase::Failure),
        "stream": keys_for_phase(registry, CallbackPhase::Stream),
        "service": keys_for_phase(registry, CallbackPhase::Service),
    })
}

fn selection(registry: &CallbackRegistry) -> Value {
    let selection = registry.select();
    json!({
        "success": selection.success,
        "failure": selection.failure,
        "success_and_failure": selection.success_and_failure,
    })
}
