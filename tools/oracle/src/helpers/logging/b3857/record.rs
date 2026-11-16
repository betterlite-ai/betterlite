use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_logging::otel::legacy_metrics::events::{EventCall, SemanticEventEmitter};
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::legacy_metrics::recorder::{LegacyMetricRecorder, MetricCall};
use betterlite_logging::otel::legacy_policy::capture::CaptureMode;
use betterlite_logging::otel::legacy_policy::config::OpenTelemetryConfig;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::events::SystemClock;
use betterlite_logging::otel::vocabulary::semconv::OTELSemconvCategory;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use chrono::FixedOffset;
use opentelemetry::logs::{AnyValue, LoggerProvider as _};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry::trace::SpanContext;
use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};

use super::input::Input;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

struct Settings(Option<OtelModelValue>);

impl CallbackSettingsView for Settings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.clone()
    }
}

pub(super) fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn capture_mode(name: Option<&str>) -> CaptureMode {
    match name {
        Some("SPAN_ONLY") => CaptureMode::SpanOnly,
        Some("EVENT_ONLY") => CaptureMode::EventOnly,
        Some("NO_CONTENT") => CaptureMode::NoContent,
        _ => CaptureMode::SpanAndEvent,
    }
}

fn seconds(time: Option<&OtelModelValue>, name: &str) -> Result<Decimal, Box<HelperError>> {
    time.and_then(OtelModelValue::as_decimal)
        .ok_or_else(|| invalid(&format!("{name} must be a number")))
}

// the string attributes carry JSON text where a container was written, and both sides read it back
fn text_json(text: &str) -> Value {
    if text.starts_with(['{', '[']) {
        return serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
    }
    Value::String(text.to_owned())
}

fn any_json(value: &AnyValue) -> Value {
    match value {
        AnyValue::Int(number) => Value::from(*number),
        AnyValue::Double(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        AnyValue::String(text) => Value::String(text.as_str().to_owned()),
        AnyValue::Boolean(flag) => Value::Bool(*flag),
        AnyValue::ListAny(items) => Value::Array(items.iter().map(any_json).collect()),
        AnyValue::Map(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, item)| (key.as_str().to_owned(), any_json(item)))
                .collect(),
        ),
        AnyValue::Bytes(_) | _ => Value::Null,
    }
}

fn attribute_json(value: &AnyValue) -> Value {
    if let AnyValue::String(text) = value {
        return text_json(text.as_str());
    }
    any_json(value)
}

fn collect_metrics(exporter: &InMemoryMetricExporter) -> Result<Vec<Value>, Box<HelperError>> {
    let finished = exporter
        .get_finished_metrics()
        .map_err(|error| invalid(&error.to_string()))?;
    let mut lines: Vec<(String, Option<String>, Value)> = Vec::new();
    for metric in finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
    {
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            continue;
        };
        for point in histogram.data_points() {
            let mut attributes = Map::new();
            let mut token_type = None;
            for pair in point.attributes() {
                let text = pair.value.to_string();
                if pair.key.as_str() == "gen_ai.token.type" {
                    token_type = Some(text.clone());
                }
                attributes.insert(pair.key.to_string(), text_json(&text));
            }
            let line = json!({
                "name": metric.name(),
                "attributes": attributes,
                "count": point.count(),
                "sum": format!("{:.9}", point.sum()),
            });
            lines.push((metric.name().to_owned(), token_type, line));
        }
    }
    lines.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
    Ok(lines.into_iter().map(|(_, _, line)| line).collect())
}

fn collect_events(input: &Input) -> Result<Vec<Value>, Box<HelperError>> {
    let exporter = InMemoryLogExporter::default();
    let provider = SdkLoggerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let emitter = SemanticEventEmitter {
        event_logger: provider.logger("b3857"),
        clock: Box::new(SystemClock),
        enable_events: input.config.enable_events.unwrap_or(true),
        semconv_latest: input.config.semconv,
    };
    let context = SpanContext::empty_context();
    emitter
        .emit_semantic_logs(&EventCall {
            kwargs: &input.kwargs,
            response: input.response.as_ref(),
            span_context: &context,
            capture_mode: capture_mode(input.config.capture_mode.as_deref()),
        })
        .map_err(|error| invalid(&error.to_string()))?;
    let recorded = exporter
        .get_emitted_logs()
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(recorded
        .iter()
        .map(|data| {
            let attributes: Map<String, Value> = data
                .record
                .attributes_iter()
                .map(|(key, value)| (key.to_string(), attribute_json(value)))
                .collect();
            json!({
                "body": data.record.body().map_or(Value::Null, any_json),
                "attributes": attributes,
            })
        })
        .collect())
}

pub(super) fn record_success(input: &Input) -> HelperResult {
    let mut config = OpenTelemetryConfig::from_env(&BTreeMap::new());
    config.attributes.clone_from(&input.config.attributes);
    if input.config.semconv {
        config
            .semconv_stability_opt_in
            .insert(OTELSemconvCategory::GenAiLatestExperimental);
    }
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let offset = FixedOffset::east_opt(0).ok_or_else(|| invalid("no UTC offset"))?;
    let recorder = LegacyMetricRecorder::new(
        create_instruments(&provider.meter("b3857")),
        &config,
        input.callback_name.clone(),
        Arc::new(Settings(input.callback_settings.clone())),
        offset,
    )
    .map_err(|error| invalid(&error.to_string()))?;
    recorder
        .record_metrics(&MetricCall {
            kwargs: &input.kwargs,
            response: input.response.as_ref(),
            start_time: seconds(input.start_time.as_ref(), "start_time")?,
            end_time: seconds(input.end_time.as_ref(), "end_time")?,
        })
        .map_err(|error| invalid(&error.to_string()))?;
    let events = collect_events(input)?;
    provider
        .force_flush()
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(json!({"metrics": collect_metrics(&exporter)?, "events": events}))
}
