use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::legacy_metrics::recorder::MetricCall;
use crate::otel::logger::call_context::{CallContext, SpanTimes};
use crate::otel::logger::finish::{close_llm_call, emit_mcp};
use crate::otel::logger::pre_call::log_pre_api_call as emit_pre_call;
use crate::otel::logger::v2::OpenTelemetryV2;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::metadata::wire::{MetadataError, MetadataMap};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::plumbing::metrics::recorder::GenAiMetricRecorder;

#[async_trait]
impl CallbackLogger for OpenTelemetryV2 {
    fn callback_name(&self) -> &str {
        &self.name
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        dispatch(self, &event, false)
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        dispatch(self, &event, true)
    }
}

impl ApiCallLogger for OpenTelemetryV2 {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        emit_pre_call(self, event)
    }

    // the boundary span spans the whole call and closes on the success or failure callback
    fn log_post_api_call(&self, _event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

fn dispatch(
    logger: &OpenTelemetryV2,
    event: &LoggerEvent,
    failure: bool,
) -> Result<(), LoggerError> {
    let api_start = {
        let calls = logger.open_calls();
        event
            .payload
            .litellm_call_id
            .as_deref()
            .and_then(|id| calls.calls.get(id))
            .map(|call| call.start_time)
    };
    let call =
        CallContext::from_event(event, api_start).map_err(|error| delivery(logger, &error))?;
    let times = SpanTimes::from_payload(&event.payload);
    if emit_mcp(logger, &call, times).map_err(|error| delivery(logger, &error))? {
        return Ok(());
    }
    close_llm_call(logger, &call, times).map_err(|error| delivery(logger, &error))?;
    record(logger, &call, failure).map_err(|error| delivery(logger, &error))
}

fn delivery(logger: &OpenTelemetryV2, error: &dyn Display) -> LoggerError {
    LoggerError::Delivery {
        callback: logger.name.clone(),
        message: error.to_string(),
    }
}

// litellm.integrations.otel.logger.OpenTelemetryV2._record_metrics and _record_failure_metrics: a synthetic proxy-gate log records no failure metric
fn record(
    logger: &OpenTelemetryV2,
    call: &CallContext,
    failure: bool,
) -> Result<(), MetadataError> {
    if failure && call.event.is_no_upstream_call {
        return Ok(());
    }
    let Some(payload) = call.event.payload.as_ref() else {
        return Ok(());
    };
    let kwargs = metric_kwargs(call, payload)?;
    if failure {
        guarded_record(logger, |recorder| {
            recorder.record_failure(&kwargs, call.start_time, call.end_time)
        });
        return Ok(());
    }
    let recorded: OtelModelValue =
        serde_json::from_value(serde_json::to_value(&payload.response)?)?;
    // a JSON null response is a response the call never had; the recorder reads no metric out of either
    let response = match &recorded {
        OtelModelValue::Null(()) => None,
        OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_)
        | OtelModelValue::Array(_)
        | OtelModelValue::Object(_) => Some(&recorded),
    };
    guarded_record(logger, |recorder| {
        recorder.record(&MetricCall {
            kwargs: &kwargs,
            response,
            start_time: call.start_time,
            end_time: call.end_time,
        })
    });
    Ok(())
}

// litellm.integrations.otel.logger.OpenTelemetryV2._guarded_record: an operator-fixable filter error surfaces once at ERROR
fn guarded_record<F>(logger: &OpenTelemetryV2, record: F)
where
    F: FnOnce(&GenAiMetricRecorder) -> Result<(), LegacyMetricsError>,
{
    let Some(recorder) = logger.metrics.as_ref() else {
        return;
    };
    match record(recorder) {
        Ok(()) => {}
        Err(error) if is_filter_error(&error) => {
            if !logger
                .metric_filter_error_logged
                .swap(true, Ordering::Relaxed)
            {
                tracing::error!(
                    %error,
                    "OpenTelemetryV2: invalid otel.attributes metric filter, metrics disabled"
                );
            }
        }
        Err(error) => tracing::debug!(%error, "OpenTelemetryV2: metric recording failed"),
    }
}

// the errors an otel.attributes filter raises are LiteLLM's ValueError class
const fn is_filter_error(error: &LegacyMetricsError) -> bool {
    matches!(
        error,
        LegacyMetricsError::AttributesNotMapping
            | LegacyMetricsError::AttributesShape { .. }
            | LegacyMetricsError::BothLists
            | LegacyMetricsError::TokenTypeFiltered
            | LegacyMetricsError::UnknownAttributeNames { .. }
    )
}

// the kwargs the recorder reads, as LiteLLM's kwargs carries them
fn metric_kwargs(call: &CallContext, payload: &Payload) -> Result<MetadataMap, MetadataError> {
    let mut kwargs: MetadataMap = serde_json::from_value(serde_json::to_value(payload)?)?;
    kwargs.insert(
        "standard_logging_object".to_owned(),
        OtelModelValue::Object(kwargs.clone()),
    );
    kwargs.insert("litellm_params".to_owned(), litellm_params(payload)?);
    kwargs.insert(
        "optional_params".to_owned(),
        OtelModelValue::Object(BTreeMap::from([(
            "stream".to_owned(),
            OtelModelValue::Bool(call.stream),
        )])),
    );
    stamp_times(&mut kwargs, call);
    Ok(kwargs)
}

// the provider and metadata LiteLLM's kwargs carries under litellm_params
fn litellm_params(payload: &Payload) -> Result<OtelModelValue, MetadataError> {
    Ok(OtelModelValue::Object(BTreeMap::from([
        (
            "custom_llm_provider".to_owned(),
            payload
                .extra
                .get("custom_llm_provider")
                .cloned()
                .unwrap_or(OtelModelValue::Null(())),
        ),
        (
            "metadata".to_owned(),
            serde_json::from_value(serde_json::to_value(&payload.metadata)?)?,
        ),
    ])))
}

// the latency stamps the recorder reads, present only when the call carried a positive one
fn stamp_times(kwargs: &mut MetadataMap, call: &CallContext) {
    for (key, value) in [
        ("api_call_start_time", call.api_call_start_time),
        ("completion_start_time", call.completion_start_time),
    ] {
        if value > Decimal::ZERO
            && let Some(number) = value.to_f64().and_then(Number::from_f64)
        {
            kwargs.insert(key.to_owned(), OtelModelValue::Number(number));
        }
    }
}
