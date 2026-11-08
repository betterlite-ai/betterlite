use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::callbacks::{CallbackKind, should_run_callback};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup, restore_correlation_context,
    restore_correlation_guarded,
};
use betterlite_logging::logging_state::events::{
    LoggingEvent, claim_async_success_log, has_run_logging, is_sync_litellm_request,
    set_response_timing_metrics, should_run_logging, update_completion_start_time,
};
use betterlite_logging::logging_state::updates::{
    CallUpdate, update_environment_variables, update_from_kwargs, update_messages,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};

mod snapshot;

const DEFAULT_GENERATED_TRACE_ID: &str = "00000000-0000-4000-8000-000000000000";

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    instances: Vec<Instance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Instance {
    init: LoggingInit,
    #[serde(default)]
    correlation: CorrelationInput,
    #[serde(default)]
    operations: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct CorrelationInput {
    context: CorrelationContext,
    settings: CorrelationSettings,
    generated_trace_id: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SetResponseTimingMetrics,
    UpdateEnvironmentVariables,
    UpdateFromKwargs,
    UpdateMessages,
    IsSyncLitellmRequest,
    ShouldRunLogging,
    HasRunLogging,
    ClaimAsyncSuccessLog,
    ShouldRunCallback,
    UpdateCompletionStartTime,
    RestoreCorrelation,
    RestoreCorrelationGuarded,
    SetCorrelation,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    op: Operation,
    #[serde(default)]
    metrics: IndexMap<String, f64>,
    #[serde(default)]
    litellm_params: IndexMap<String, Json>,
    #[serde(default)]
    update: CallUpdate,
    #[serde(default)]
    kwargs: IndexMap<String, Json>,
    #[serde(default)]
    messages: Vec<Json>,
    #[serde(default)]
    event: Option<LoggingEvent>,
    #[serde(default)]
    callback: CallbackKind,
    #[serde(default)]
    completion_start_time: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    trace_id: String,
    #[serde(default)]
    session_id: String,
}

impl Default for CorrelationInput {
    fn default() -> Self {
        Self {
            context: CorrelationContext::default(),
            settings: CorrelationSettings::default(),
            generated_trace_id: DEFAULT_GENERATED_TRACE_ID.to_owned(),
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|source| failure("RedactorError", source.to_string()))?;
        let mut instances = Vec::new();
        for instance in &input.instances {
            instances.push(run_instance(instance, &redactor)?);
        }
        Ok(serde_json::json!({ "instances": instances }))
    }
}

fn run_instance(instance: &Instance, redactor: &Redactor) -> Result<Value, Box<HelperError>> {
    let mut context = instance.correlation.context.clone();
    let mut state = LoggingState::new(
        instance.init.clone(),
        CorrelationSetup {
            context: &mut context,
            settings: instance.correlation.settings,
        },
        redactor,
        || instance.correlation.generated_trace_id.clone(),
    )
    .map_err(|error| failure("LoggingStateError", error.to_string()))?;
    let initial = snapshot::snapshot(&state, &context);
    let mut results = Vec::new();
    for step in &instance.operations {
        results.push(apply_operation(&mut state, &mut context, step)?);
    }
    Ok(serde_json::json!({ "init": initial, "results": results }))
}

fn apply_operation(
    state: &mut LoggingState,
    context: &mut CorrelationContext,
    step: &Step,
) -> Result<Value, Box<HelperError>> {
    match step.op {
        Operation::SetResponseTimingMetrics => Ok(timing(state, step)),
        Operation::UpdateEnvironmentVariables => {
            environment(state, step)?;
            Ok(snapshot::snapshot(state, context))
        }
        Operation::UpdateFromKwargs => {
            from_kwargs(state, step)?;
            Ok(snapshot::snapshot(state, context))
        }
        Operation::UpdateMessages => {
            update_messages(state, step.messages.clone());
            Ok(snapshot::snapshot(state, context))
        }
        Operation::IsSyncLitellmRequest => Ok(serde_json::json!(is_sync_litellm_request(
            &step.litellm_params
        ))),
        Operation::ShouldRunLogging => Ok(serde_json::json!(should_run_logging(
            state,
            required_event(step)?
        ))),
        Operation::HasRunLogging => {
            has_run_logging(state, required_event(step)?);
            Ok(Value::Null)
        }
        Operation::ClaimAsyncSuccessLog => Ok(serde_json::json!(claim_async_success_log(state))),
        Operation::ShouldRunCallback => Ok(serde_json::json!(should_run_callback(
            state,
            step.callback,
            &step.litellm_params
        ))),
        Operation::UpdateCompletionStartTime => {
            completion(state, step)?;
            Ok(snapshot::snapshot(state, context))
        }
        Operation::RestoreCorrelation => Ok(restore(state, context)),
        Operation::RestoreCorrelationGuarded => Ok(restore_guarded(state, context)),
        Operation::SetCorrelation => Ok(set_correlation(context, step)),
    }
}

fn timing(state: &mut LoggingState, step: &Step) -> Value {
    set_response_timing_metrics(state, step.metrics.clone());
    serde_json::json!({ "response_timing_metrics": &state.response_timing_metrics })
}

fn environment(state: &mut LoggingState, step: &Step) -> Result<(), Box<HelperError>> {
    update_environment_variables(state, &step.litellm_params, step.update.clone())
        .map_err(|error| failure("LoggingStateError", error.to_string()))
}

fn from_kwargs(state: &mut LoggingState, step: &Step) -> Result<(), Box<HelperError>> {
    let caller = (!step.litellm_params.is_empty()).then_some(&step.litellm_params);
    update_from_kwargs(state, &step.kwargs, caller, step.update.clone())
        .map_err(|error| failure("LoggingStateError", error.to_string()))
}

fn completion(state: &mut LoggingState, step: &Step) -> Result<(), Box<HelperError>> {
    let Some(completion_start_time) = step.completion_start_time else {
        return Err(failure(
            "InvalidInput",
            "the completion_start_time field is required".to_owned(),
        ));
    };
    update_completion_start_time(state, completion_start_time);
    Ok(())
}

fn restore(state: &LoggingState, context: &mut CorrelationContext) -> Value {
    restore_correlation_context(state, context);
    snapshot::context(context)
}

fn restore_guarded(state: &LoggingState, context: &mut CorrelationContext) -> Value {
    restore_correlation_guarded(state, context);
    snapshot::context(context)
}

fn set_correlation(context: &mut CorrelationContext, step: &Step) -> Value {
    context.trace_id.clone_from(&step.trace_id);
    context.session_id.clone_from(&step.session_id);
    snapshot::context(context)
}

fn required_event(step: &Step) -> Result<LoggingEvent, Box<HelperError>> {
    let Some(event) = step.event else {
        return Err(failure(
            "InvalidInput",
            "the event field is required".to_owned(),
        ));
    };
    Ok(event)
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
