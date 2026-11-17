use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::FixedOffset;
use opentelemetry::KeyValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::attributes::bounded_attributes;
use super::error_type::resolve_error_type;
use super::filter::warn_about_ineligible_names;
use crate::logging_privacy::internal::unbilled_call;
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_metrics::attributes::CommonAttributes;
use crate::otel::legacy_metrics::error::LegacyMetricsError;
use crate::otel::legacy_metrics::filter::{
    CallbackSettingsView, MetricFilterState, TOKEN_TYPE_ATTRIBUTE,
};
use crate::otel::legacy_metrics::latency::{
    LatencyContext, record_api_latency_metric, record_seconds, record_tpot_metric,
    record_ttft_metric,
};
use crate::otel::legacy_metrics::recorder::MetricCall;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::model::utils::time::FixedLocalTimeResolver;
use crate::otel::vocabulary::constants::attribute_keys::error;
use crate::otel_provider_assembly::metric_instruments::MetricInstruments;

pub struct GenAiMetricRecorder {
    metrics: MetricInstruments,
    filter: MetricFilterState,
    warned: AtomicBool,
    resolver: FixedLocalTimeResolver,
}

impl GenAiMetricRecorder {
    // the proxy fills callback_settings.otel.attributes after the logger exists, so the filter waits for the first record
    pub fn new(
        metrics: MetricInstruments,
        callback_name: Option<String>,
        callback_settings: Arc<dyn CallbackSettingsView>,
        utc_offset: FixedOffset,
    ) -> Result<Self, LegacyMetricsError> {
        Ok(Self {
            metrics,
            filter: MetricFilterState::new(None, callback_name, callback_settings)?,
            warned: AtomicBool::new(false),
            resolver: FixedLocalTimeResolver { offset: utc_offset },
        })
    }

    pub fn record(&self, call: &MetricCall<'_>) -> Result<(), LegacyMetricsError> {
        let attributes = self.filter_attributes(bounded_attributes(call.kwargs)?)?;
        let duration = duration_of(call.start_time, call.end_time)?;
        let call_type = call
            .kwargs
            .get("call_type")
            .and_then(OtelModelValue::as_str);
        let params = call
            .kwargs
            .get("litellm_params")
            .and_then(OtelModelValue::as_object);
        let unbilled = unbilled_call(call_type, params, call.response);
        record_seconds(&self.metrics.operation_duration, duration, &attributes);
        if !unbilled {
            self.record_token_usage(call.response, &attributes)?;
        }
        self.record_cost(call.kwargs, &attributes)?;
        self.record_latency(call, unbilled, &attributes, duration)
    }

    pub fn record_failure(
        &self,
        kwargs: &MetadataMap,
        start_time: Decimal,
        end_time: Decimal,
    ) -> Result<(), LegacyMetricsError> {
        let mut attributes = self.filter_attributes(bounded_attributes(kwargs)?)?;
        attributes.push(KeyValue::new(error::TYPE, resolve_error_type(kwargs)?));
        let duration = duration_of(start_time, end_time)?;
        record_seconds(&self.metrics.operation_duration, duration, &attributes);
        Ok(())
    }

    fn filter_attributes(
        &self,
        attributes: CommonAttributes,
    ) -> Result<Vec<KeyValue>, LegacyMetricsError> {
        let resolved = self.filter.ensure()?;
        if !self.warned.swap(true, Ordering::Relaxed) {
            warn_about_ineligible_names(&resolved);
        }
        self.filter.filter_attributes(attributes)
    }

    fn record_cost(
        &self,
        kwargs: &MetadataMap,
        attributes: &[KeyValue],
    ) -> Result<(), LegacyMetricsError> {
        let Some(cost) = kwargs
            .get("response_cost")
            .filter(|cost| !matches!(cost, OtelModelValue::Null(())))
        else {
            return Ok(());
        };
        let amount = decimal_field(cost, "response_cost")?;
        if amount > Decimal::ZERO {
            record_seconds(&self.metrics.usage_in_usd, amount, attributes);
        }
        Ok(())
    }

    fn record_token_usage(
        &self,
        response: Option<&OtelModelValue>,
        attributes: &[KeyValue],
    ) -> Result<(), LegacyMetricsError> {
        let Some(usage) = response
            .filter(|value| is_truthy(value))
            .and_then(OtelModelValue::as_object)
            .and_then(|object| object.get("usage"))
            .filter(|usage| is_truthy(usage))
            .and_then(OtelModelValue::as_object)
        else {
            return Ok(());
        };
        for (token_type, field) in [("input", "prompt_tokens"), ("output", "completion_tokens")] {
            let count = token_count(usage, field)?.unwrap_or(Decimal::ZERO);
            let typed: Vec<KeyValue> = attributes
                .iter()
                .cloned()
                .chain([KeyValue::new(TOKEN_TYPE_ATTRIBUTE, token_type)])
                .collect();
            if let Some(number) = count.to_f64() {
                self.metrics.token_usage.record(number, &typed);
            }
        }
        Ok(())
    }

    fn record_latency(
        &self,
        call: &MetricCall<'_>,
        unbilled: bool,
        attributes: &[KeyValue],
        duration: Decimal,
    ) -> Result<(), LegacyMetricsError> {
        let context = LatencyContext {
            kwargs: call.kwargs,
            resolver: &self.resolver,
            attributes,
            end_time: call.end_time,
            duration,
        };
        record_ttft_metric(&self.metrics.time_to_first_token, &context)?;
        if !unbilled {
            self.record_time_per_output_token(call, &context)?;
        }
        record_api_latency_metric(&self.metrics.response_duration, &context)
    }

    fn record_time_per_output_token(
        &self,
        call: &MetricCall<'_>,
        context: &LatencyContext<'_>,
    ) -> Result<(), LegacyMetricsError> {
        let Some(usage) = call
            .response
            .and_then(OtelModelValue::as_object)
            .and_then(|object| object.get("usage"))
            .filter(|usage| is_truthy(usage))
            .and_then(OtelModelValue::as_object)
        else {
            return Ok(());
        };
        let Some(count) = token_count(usage, "completion_tokens")? else {
            return Ok(());
        };
        if count <= Decimal::ZERO {
            return Ok(());
        }
        record_tpot_metric(&self.metrics.time_per_output_token, context, count)
    }
}

fn token_count(
    usage: &MetadataMap,
    field: &'static str,
) -> Result<Option<Decimal>, LegacyMetricsError> {
    match usage.get(field) {
        None | Some(OtelModelValue::Null(())) => Ok(None),
        Some(value) => decimal_field(value, field).map(Some),
    }
}

fn duration_of(start_time: Decimal, end_time: Decimal) -> Result<Decimal, LegacyMetricsError> {
    end_time
        .checked_sub(start_time)
        .ok_or(LegacyMetricsError::TimeRange)
}

fn decimal_field(
    value: &OtelModelValue,
    field: &'static str,
) -> Result<Decimal, LegacyMetricsError> {
    value.as_decimal().ok_or(LegacyMetricsError::WrongType {
        field,
        expected: "a number",
    })
}
