use std::sync::Arc;

use chrono::FixedOffset;
use opentelemetry::KeyValue;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::attributes::common_attributes;
use super::error::LegacyMetricsError;
use super::filter::{CallbackSettingsView, MetricFilterState, TOKEN_TYPE_ATTRIBUTE};
use super::latency::{
    LatencyContext, record_api_latency_metric, record_seconds, record_tpot_metric,
    record_ttft_metric,
};
use crate::logging_privacy::internal::{Origin, call_category, is_unbilled};
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::legacy_policy::semconv_attrs::semconv_latest_experimental;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::model::utils::time::FixedLocalTimeResolver;
use crate::otel_provider_assembly::metric_instruments::MetricInstruments;

mod tests;

const BACKGROUND_COST_POLL_ORIGIN: &str = "background_response_cost_poll";

pub struct LegacyMetricRecorder {
    instruments: MetricInstruments,
    filter: MetricFilterState,
    semconv_latest: bool,
    resolver: FixedLocalTimeResolver,
}

// start_time and end_time are epoch seconds, the way LiteLLM's datetimes.timestamp() reads them
pub struct MetricCall<'call> {
    pub kwargs: &'call MetadataMap,
    pub response: Option<&'call OtelModelValue>,
    pub start_time: Decimal,
    pub end_time: Decimal,
}

impl LegacyMetricRecorder {
    pub fn new(
        instruments: MetricInstruments,
        config: &OpenTelemetryConfig,
        callback_name: Option<String>,
        callback_settings: Arc<dyn CallbackSettingsView>,
        utc_offset: FixedOffset,
    ) -> Result<Self, LegacyMetricsError> {
        Ok(Self {
            instruments,
            filter: MetricFilterState::new(
                config.attributes.clone(),
                callback_name,
                callback_settings,
            )?,
            semconv_latest: semconv_latest_experimental(config),
            resolver: FixedLocalTimeResolver { offset: utc_offset },
        })
    }

    pub fn record_metrics(&self, call: &MetricCall<'_>) -> Result<(), LegacyMetricsError> {
        let duration = call
            .end_time
            .checked_sub(call.start_time)
            .ok_or(LegacyMetricsError::TimeRange)?;
        let attributes = self
            .filter
            .filter_attributes(common_attributes(call.kwargs, self.semconv_latest)?)?;
        record_seconds(&self.instruments.operation_duration, duration, &attributes);
        let unbilled = is_unbilled_call(call.kwargs, call.response);
        let usage = call
            .response
            .filter(|response| is_truthy(response))
            .filter(|_| !unbilled)
            .and_then(OtelModelValue::as_object)
            .and_then(|response| response.get("usage"))
            .filter(|usage| is_truthy(usage))
            .and_then(OtelModelValue::as_object);
        if let Some(counts) = usage {
            self.record_token_usage(counts, &attributes)?;
        }
        if let Some(cost) = call
            .kwargs
            .get("response_cost")
            .filter(|cost| is_truthy(cost))
        {
            let amount = decimal_field(cost, "response_cost")?;
            record_seconds(&self.instruments.usage_in_usd, amount, &attributes);
        }
        let context = LatencyContext {
            kwargs: call.kwargs,
            resolver: &self.resolver,
            attributes: &attributes,
            end_time: call.end_time,
            duration,
        };
        record_ttft_metric(&self.instruments.time_to_first_token, &context)?;
        self.record_per_output_token(call, unbilled, &context)?;
        record_api_latency_metric(&self.instruments.response_duration, &context)
    }

    fn record_token_usage(
        &self,
        usage: &MetadataMap,
        attributes: &[KeyValue],
    ) -> Result<(), LegacyMetricsError> {
        for (token_type, field) in [("input", "prompt_tokens"), ("output", "completion_tokens")] {
            let count = match usage.get(field) {
                None => Decimal::ZERO,
                Some(value) => decimal_field(value, field)?,
            };
            let mut typed = attributes.to_vec();
            typed.push(KeyValue::new(TOKEN_TYPE_ATTRIBUTE, token_type));
            if let Some(number) = count.to_f64() {
                self.instruments.token_usage.record(number, &typed);
            }
        }
        Ok(())
    }

    fn record_per_output_token(
        &self,
        call: &MetricCall<'_>,
        unbilled: bool,
        context: &LatencyContext<'_>,
    ) -> Result<(), LegacyMetricsError> {
        let tokens = call
            .response
            .and_then(OtelModelValue::as_object)
            .and_then(|response| response.get("usage"))
            .filter(|usage| is_truthy(usage))
            .and_then(OtelModelValue::as_object)
            .and_then(|usage| usage.get("completion_tokens"))
            .filter(|tokens| !matches!(tokens, OtelModelValue::Null(())));
        let Some(completion_tokens) = tokens.filter(|_| !unbilled) else {
            return Ok(());
        };
        let count = decimal_field(completion_tokens, "completion_tokens")?;
        if count <= Decimal::ZERO {
            return Ok(());
        }
        record_tpot_metric(&self.instruments.time_per_output_token, context, count)
    }
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

// a read or management call replays usage a create already billed, unless it polls a background response
fn is_unbilled_call(kwargs: &MetadataMap, response: Option<&OtelModelValue>) -> bool {
    let call_type = kwargs.get("call_type").and_then(OtelModelValue::as_str);
    let background = response
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("background"))
        .and_then(OtelModelValue::as_bool)
        == Some(true);
    is_unbilled(
        call_category(call_type),
        cost_poll_origin(kwargs),
        background,
    )
}

// metadata wins over litellm_metadata key by key, and only the cost-poll origin matters here
fn cost_poll_origin(kwargs: &MetadataMap) -> Option<Origin> {
    let params = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)?;
    let origin = ["metadata", "litellm_metadata"].iter().find_map(|name| {
        params
            .get(*name)
            .and_then(OtelModelValue::as_object)
            .and_then(|fields| fields.get("internal_call_origin"))
    });
    let named = origin.and_then(OtelModelValue::as_str)?;
    (named == BACKGROUND_COST_POLL_ORIGIN).then_some(Origin::BackgroundResponseCostPoll)
}
