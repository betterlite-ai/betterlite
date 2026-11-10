use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::logging_state::LoggingInit;
use betterlite_logging::logging_state::correlation::{CorrelationContext, CorrelationSettings};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

use super::fixtures::LoggerSpec;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) instances: Vec<Instance>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Instance {
    pub(crate) init: LoggingInit,
    #[serde(default)]
    pub(crate) correlation: CorrelationInput,
    pub(crate) failure: FailureInput,
    #[serde(default)]
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) loggers: Vec<LoggerSpec>,
    #[serde(default)]
    pub(crate) clock: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub(crate) operations: Vec<Step>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CorrelationInput {
    pub(crate) context: CorrelationContext,
    pub(crate) settings: CorrelationSettings,
    pub(crate) generated_trace_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureInput {
    pub(crate) exception: ExceptionFacts,
    #[serde(default)]
    pub(crate) traceback: String,
    #[serde(default)]
    pub(crate) start_time: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub(crate) end_time: Option<DateTime<FixedOffset>>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    RecordPartialUsage,
    RecordAssembledResponse,
    FailureHandlerHelperFn,
    FailureHandler,
    AsyncFailureHandler,
    DispatchFailureHandlers,
}

impl Operation {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::RecordPartialUsage => "record_partial_usage",
            Self::RecordAssembledResponse => "record_assembled_response",
            Self::FailureHandlerHelperFn => "failure_handler_helper_fn",
            Self::FailureHandler => "failure_handler",
            Self::AsyncFailureHandler => "async_failure_handler",
            Self::DispatchFailureHandlers => "dispatch_failure_handlers",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    pub(crate) op: Operation,
    #[serde(default)]
    pub(crate) usage: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub(crate) cost: Option<Decimal>,
    #[serde(default)]
    pub(crate) prefer_async_handlers: bool,
}
