use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_logging::dynamic_callbacks::DynamicCallbackLists;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::logging_failure::callbacks::FailureCallbacks;
use betterlite_logging::logging_failure::handlers::async_failure_handler;
use betterlite_logging::logging_failure::handlers::dispatch_failure_handlers;
use betterlite_logging::logging_failure::handlers::failure_handler;
use betterlite_logging::logging_failure::helper_fn::failure_handler_helper_fn;
use betterlite_logging::logging_failure::partial_usage::AssembledResponse;
// ast-grep-ignore: no-long-names
use betterlite_logging::logging_failure::partial_usage::record_assembled_response_for_failure;
// ast-grep-ignore: no-long-names
use betterlite_logging::logging_failure::partial_usage::record_partial_usage_for_failure;
use betterlite_logging::logging_failure::{FailureCall, FailureDeps, FailureRecord, FailureReport};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{CorrelationContext, CorrelationSetup};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde_json::{Value, json};

use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};

use self::fixtures::{Fakes, lock};
use self::input::{Input, Instance, Operation, Step};
use self::snapshot::{context as context_snapshot, details};

mod fixtures;
mod input;
mod snapshot;

pub(crate) struct Op;

struct Applied {
    timing: Option<Value>,
}

struct Assembled {
    usage: Option<Usage>,
}

struct Environment<'run> {
    call: FailureCall<'run>,
    callbacks: FailureCallbacks<'run>,
    deps: FailureDeps<'run>,
    fakes: &'run Fakes,
}

impl AssembledResponse for Assembled {
    fn usage(&self) -> Option<&Usage> {
        self.usage.as_ref()
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        runtime.block_on(run_instances(input))
    }
}

async fn run_instances(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut instances = Vec::new();
    for instance in &input.instances {
        instances.push(run_instance(instance).await?);
    }
    Ok(json!({ "instances": instances }))
}

async fn run_instance(instance: &Instance) -> Result<Value, Box<HelperError>> {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| failure("RedactorError", error.to_string()))?;
    let mut correlation = instance.correlation.context.clone();
    let mut state = build_state(instance, &redactor, &mut correlation)?;
    let dynamic = DynamicCallbackLists::from(&instance.init.dynamic_callbacks);
    let fakes = Fakes::new(instance)?;
    let deps = fakes.deps(&redactor);
    let callbacks = FailureCallbacks {
        registry: &fakes.registry,
        dynamic: Some(&dynamic),
    };
    let report = FailureReport {
        exception: Arc::new(instance.failure.exception.clone()),
        traceback: instance.failure.traceback.clone(),
        start_time: instance.failure.start_time,
        end_time: instance.failure.end_time,
    };
    let mut record = FailureRecord::default();
    let mut environment = Environment {
        call: FailureCall {
            state: &mut state,
            record: &mut record,
            context: &mut correlation,
        },
        callbacks,
        deps,
        fakes: &fakes,
    };
    let steps = run_steps(&mut environment, &report, &instance.operations).await?;
    Ok(json!({ "steps": steps }))
}

fn build_state(
    instance: &Instance,
    redactor: &Redactor,
    correlation: &mut CorrelationContext,
) -> Result<LoggingState, Box<HelperError>> {
    let generated = instance.correlation.generated_trace_id.clone();
    LoggingState::new(
        instance.init.clone(),
        CorrelationSetup {
            context: correlation,
            settings: instance.correlation.settings,
        },
        redactor,
        || {
            generated
                .clone()
                .unwrap_or_else(|| "generated-trace".to_owned())
        },
        || Duration::ZERO,
    )
    .map_err(|error| failure("LoggingStateError", error.to_string()))
}

async fn run_steps(
    environment: &mut Environment<'_>,
    report: &FailureReport,
    steps: &[Step],
) -> Result<Vec<Value>, Box<HelperError>> {
    let mut results = Vec::new();
    for step in steps {
        let calls_before = lock(&environment.fakes.calls).len();
        let applied = apply(
            &mut environment.call,
            &environment.callbacks,
            &environment.deps,
            step,
            report,
        )
        .await?;
        results.push(json!({
            "op": step.op.name(),
            "details": details(environment.call.state),
            "context": context_snapshot(environment.call.context),
            "calls": environment.fakes.calls_since(calls_before),
            "executor_jobs": environment.fakes.jobs.load(Ordering::SeqCst),
            "timing": applied.timing,
        }));
    }
    Ok(results)
}

async fn apply(
    call: &mut FailureCall<'_>,
    callbacks: &FailureCallbacks<'_>,
    deps: &FailureDeps<'_>,
    step: &Step,
    report: &FailureReport,
) -> Result<Applied, Box<HelperError>> {
    match step.op {
        Operation::RecordPartialUsage | Operation::RecordAssembledResponse => {
            record_usage(call, step)
        }
        Operation::FailureHandlerHelperFn => {
            let timing = failure_handler_helper_fn(call, report, deps)
                .map_err(|error| failure("FailureError", error.to_string()))?;
            Ok(Applied {
                timing: Some(json!({
                    "start_micros": timing.start_time.timestamp_micros(),
                    "end_micros": timing.end_time.timestamp_micros(),
                })),
            })
        }
        Operation::FailureHandler => {
            failure_handler(call, report, callbacks, deps);
            Ok(Applied { timing: None })
        }
        Operation::AsyncFailureHandler => {
            async_failure_handler(call, report, callbacks, deps).await;
            Ok(Applied { timing: None })
        }
        Operation::DispatchFailureHandlers => {
            dispatch_failure_handlers(call, report, callbacks, deps, step.prefer_async_handlers)
                .await;
            Ok(Applied { timing: None })
        }
    }
}

fn record_usage(call: &mut FailureCall<'_>, step: &Step) -> Result<Applied, Box<HelperError>> {
    let usage = match &step.usage {
        Some(mapping) => Some(
            usage_from_mapping(mapping)
                .map_err(|error| failure("UsageError", error.to_string()))?,
        ),
        None => None,
    };
    if matches!(step.op, Operation::RecordPartialUsage) {
        let Some(found) = usage.as_ref() else {
            return Err(failure(
                "InvalidInput",
                "the usage field is required".to_owned(),
            ));
        };
        let cost = step
            .cost
            .ok_or_else(|| failure("InvalidInput", "the cost field is required".to_owned()))?;
        // ast-grep-ignore: no-long-names
        record_partial_usage_for_failure(call.state, found, cost);
    } else {
        let assembled = Assembled { usage };
        // ast-grep-ignore: no-long-names
        record_assembled_response_for_failure(call.state, &assembled, |_| step.cost);
    }
    Ok(Applied { timing: None })
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
