use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_metrics::filter::{
    CallbackSettingsView, TOKEN_TYPE_ATTRIBUTE,
};
use betterlite_logging::otel::legacy_metrics::recorder::MetricCall;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use chrono::FixedOffset;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, HistogramDataPoint, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};

use super::input::{Function, Input};

struct Settings(Option<OtelModelValue>);

impl CallbackSettingsView for Settings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.clone()
    }
}

#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error("{0} must be a number")]
    Time(&'static str),
    #[error("no utc offset")]
    Offset,
}

struct Harness {
    recorder: GenAiMetricRecorder,
    provider: SdkMeterProvider,
    exporter: InMemoryMetricExporter,
}

pub(super) fn record_call(input: &Input) -> HelperResult {
    let harness = prepare(input)?;
    let start = seconds(input.start_time.as_ref(), "start_time")?;
    let end = seconds(input.end_time.as_ref(), "end_time")?;
    match input.function {
        Function::Record => harness.recorder.record(&MetricCall {
            kwargs: &input.kwargs,
            response: input.response.as_ref(),
            start_time: start,
            end_time: end,
        }),
        Function::RecordFailure => harness.recorder.record_failure(&input.kwargs, start, end),
    }
    .or_value_error()?;
    collected(&harness.provider, &harness.exporter)
}

fn prepare(input: &Input) -> Result<Harness, Box<HelperError>> {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&provider.meter("b3869")),
        input.callback_name.clone(),
        Arc::new(Settings(input.callback_settings.clone())),
        FixedOffset::east_opt(0)
            .ok_or(InputError::Offset)
            .or_value_error()?,
    )
    .or_value_error()?;
    Ok(Harness {
        recorder,
        provider,
        exporter,
    })
}

fn seconds(time: Option<&OtelModelValue>, name: &'static str) -> Result<Decimal, Box<HelperError>> {
    time.and_then(OtelModelValue::as_decimal)
        .ok_or(InputError::Time(name))
        .or_value_error()
}

// points come out of a hash map, so both sides order them by token type before comparing
fn collected(provider: &SdkMeterProvider, exporter: &InMemoryMetricExporter) -> HelperResult {
    provider.force_flush().or_value_error()?;
    let finished = exporter.get_finished_metrics().or_value_error()?;
    let mut grouped: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for metric in finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
    {
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            continue;
        };
        let mut points: Vec<Value> = histogram
            .data_points()
            .map(|point| {
                json!({
                    "attributes": point_attributes(point),
                    "count": point.count(),
                    "sum": format!("{:.9}", point.sum()),
                })
            })
            .collect();
        points.sort_by_key(token_type);
        grouped.insert(metric.name().to_owned(), points);
    }
    Ok(json!({"metrics": grouped}))
}

fn point_attributes(point: &HistogramDataPoint<f64>) -> Map<String, Value> {
    point
        .attributes()
        .map(|pair| {
            let text = pair.value.to_string();
            (pair.key.to_string(), cell(&text))
        })
        .collect()
}

// a container attribute travels as JSON text, and both sides read it back so the spacing never matters
fn cell(text: &str) -> Value {
    let container = text.starts_with(['{', '[']);
    if !container {
        return Value::String(text.to_owned());
    }
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

fn token_type(point: &Value) -> String {
    point
        .get("attributes")
        .and_then(Value::as_object)
        .and_then(|attributes| attributes.get(TOKEN_TYPE_ATTRIBUTE))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
