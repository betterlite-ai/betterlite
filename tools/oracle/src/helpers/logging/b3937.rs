#[path = "b3937/datadog_llm_metrics/tests.rs"]
mod datadog_llm_metrics;

use betterlite_logging::datadog_llm_metrics::error_info::error_info;
use betterlite_logging::datadog_llm_metrics::metrics::span_metrics;
use betterlite_logging::datadog_llm_metrics::span_kind::span_kind;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use serde::Deserialize;

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SpanKind(SpanKindInput),
    Metrics(MetricsInput),
    ErrorInfo(ErrorInfoInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpanKindInput {
    call_type: Option<String>,
    parent_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetricsInput {
    payload: Box<StandardLoggingPayload>,
    tool_output_tokens: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorInfoInput {
    payload: Box<StandardLoggingPayload>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::SpanKind(kind) => serde_json::to_value(span_kind(
                kind.call_type.as_deref(),
                kind.parent_id.as_deref(),
            ))
            .or_value_error(),
            Input::Metrics(metrics) => serde_json::to_value(
                span_metrics(&metrics.payload, metrics.tool_output_tokens).or_value_error()?,
            )
            .or_value_error(),
            Input::ErrorInfo(info) => {
                serde_json::to_value(error_info(&info.payload)).or_value_error()
            }
        }
    }
}
