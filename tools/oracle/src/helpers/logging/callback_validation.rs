use betterlite_logging::callback_config::error::{PayloadError, ValidationError};
use betterlite_logging::callback_config::family::{
    conflicting_span_scope_error, cross_entry_family_error,
};
use betterlite_logging::callback_config::options::{
    arize_sampling_rate_error, callback_config_error, langfuse_environment_error,
    langfuse_span_scope_error, newrelic_config_error,
};
use betterlite_logging::callback_config::parse::validate_logging_metadata;
use betterlite_logging::callback_config::{CallbackVars, FeatureFlags};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    CallbackConfigError,
    ArizeSamplingRateError,
    LangfuseEnvironmentError,
    LangfuseSpanScopeError,
    NewrelicConfigError,
    CrossEntryFamilyError,
    ConflictingSpanScopeError,
    LoggingMetadataConfigError,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    function: Function,
    #[serde(default)]
    callback_name: Option<String>,
    #[serde(default)]
    callback_vars: CallbackVars,
    #[serde(default)]
    stored: Vec<CallbackVars>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    otel_v2: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results: Result<Vec<Value>, Box<HelperError>> =
            input.steps.iter().map(run_step).collect();
        Ok(Value::Array(results?))
    }
}

fn run_step(step: &Step) -> Result<Value, Box<HelperError>> {
    let flags = FeatureFlags {
        otel_v2_enabled: step.otel_v2,
    };
    let name = step.callback_name.as_deref();
    let vars = &step.callback_vars;
    let outcome = match step.function {
        Function::CallbackConfigError => callback_config_error(name, vars, flags),
        Function::ArizeSamplingRateError => arize_sampling_rate_error(name, vars),
        Function::LangfuseEnvironmentError => langfuse_environment_error(vars),
        Function::LangfuseSpanScopeError => langfuse_span_scope_error(name, vars),
        Function::NewrelicConfigError => newrelic_config_error(vars, flags),
        Function::CrossEntryFamilyError => cross_entry_family_error(vars, &step.stored),
        Function::ConflictingSpanScopeError => conflicting_span_scope_error(vars, &step.stored),
        Function::LoggingMetadataConfigError => {
            let bytes = serde_json::to_vec(&step.metadata).map_err(|error| failure(&error))?;
            match validate_logging_metadata(&bytes, flags) {
                Err(PayloadError::Parse(error)) => return Err(failure(&error)),
                Err(PayloadError::Rejected(error)) => Err(error),
                Ok(()) => Ok(()),
            }
        }
    };
    Ok(match outcome {
        Ok(()) => json!({"ok": true}),
        Err(error) => error_facts(&error),
    })
}

fn failure(error: &dyn std::error::Error) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ParseError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn error_facts(error: &ValidationError) -> Value {
    let (kind, var, family) = match error {
        ValidationError::SamplingRateCallback { var, .. } => {
            ("sampling_rate_callback", Some(var.as_str()), None)
        }
        ValidationError::SamplingRateUnparsable { var, .. }
        | ValidationError::SamplingRateOutOfRange { var, .. } => {
            ("sampling_rate_value", Some(var.as_str()), None)
        }
        ValidationError::LangfuseEnvironment { .. } => ("langfuse_environment", None, None),
        ValidationError::SpanScopeCallback { .. } => ("span_scope_callback", None, None),
        ValidationError::SpanScopeValue { .. } => ("span_scope_value", None, None),
        ValidationError::NewRelicRequiresOtelV2 => ("newrelic_requires_otel_v2", None, None),
        ValidationError::NewRelicRegion { .. } => ("newrelic_region", None, None),
        ValidationError::NewRelicRegionNeedsKey => ("newrelic_region_needs_key", None, None),
        ValidationError::FamilyConflict { family, var } => (
            "family_conflict",
            Some(var.as_str()),
            Some(family.to_string()),
        ),
        ValidationError::SpanScopeConflict { .. } => ("span_scope_conflict", None, None),
    };
    json!({"error": {"kind": kind, "var": var, "family": family, "message": error.to_string()}})
}
