use std::collections::BTreeMap;
use std::fmt::Debug;
use std::mem;
use std::sync::{Arc, Mutex};

use betterlite_logging::otel::legacy_metrics::error::LegacyMetricsError;
use betterlite_logging::otel::legacy_metrics::filter::CallbackSettingsView;
use betterlite_logging::otel::legacy_metrics::recorder::MetricCall;
use betterlite_logging::otel::model::metadata::wire::MetadataMap;
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use betterlite_logging::otel::plumbing::metrics::recorder::GenAiMetricRecorder;
use betterlite_logging::otel_provider_assembly::metric_instruments::create_instruments;
use chrono::FixedOffset;
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::data::{
    AggregatedMetrics, MetricData, ResourceMetrics, ScopeMetrics,
};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

pub(super) const DURATION: &str = "gen_ai.client.operation.duration";
pub(super) const TOKENS: &str = "gen_ai.client.token.usage";
pub(super) const COST: &str = "gen_ai.usage.cost";
pub(super) const TTFT: &str = "gen_ai.server.time_to_first_token";
pub(super) const TPOT: &str = "gen_ai.server.time_per_output_token";
pub(super) const RESPONSE: &str = "gen_ai.client.response.duration";
pub(super) const BASE_KWARGS: &str = r#"{
    "model": "gpt-4o",
    "call_type": "acompletion",
    "litellm_params": {"custom_llm_provider": "openai", "metadata": {}},
    "standard_logging_object": {
        "metadata": {"user_api_key_alias": "key-a"},
        "hidden_params": {"model_id": "deployment-1"}
    }
}"#;
pub(super) const USAGE_2_4: &str = r#"{"usage": {"prompt_tokens": 2, "completion_tokens": 4}}"#;
pub(super) const USAGE_1_4: &str = r#"{"usage": {"prompt_tokens": 1, "completion_tokens": 4}}"#;

#[derive(Debug)]
pub(super) struct Recorded {
    pub(super) name: String,
    pub(super) attributes: BTreeMap<String, String>,
    pub(super) count: u64,
    pub(super) sum: String,
}

pub(super) struct NoSettings;

impl CallbackSettingsView for NoSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        None
    }
}

// the proxy writes callback_settings.otel.attributes after the logger exists, so the filter reads it on the first record
pub(super) struct LateSettings(Mutex<Option<OtelModelValue>>);

impl CallbackSettingsView for LateSettings {
    fn otel_attributes(&self) -> Option<OtelModelValue> {
        self.0.lock().expect("settings").clone()
    }
}

impl LateSettings {
    pub(super) fn set(&self, attributes: Option<&str>) {
        *self.0.lock().expect("settings") = attributes.map(typed);
    }
}

pub(super) fn late_settings(attributes: Option<&str>) -> Arc<LateSettings> {
    Arc::new(LateSettings(Mutex::new(attributes.map(typed))))
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<String>>>);

impl Captured {
    pub(super) fn take(&self) -> Vec<String> {
        match self.0.lock() {
            Ok(mut messages) => mem::take(&mut messages),
            Err(poisoned) => mem::take(&mut poisoned.into_inner()),
        }
    }
}

struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Captured {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        self.0.lock().expect("messages").push(message.0);
    }
}

pub(super) fn capturing<T, F>(call: F) -> (T, Captured)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let subscriber = registry().with(captured.clone());
    let value = with_default(subscriber, call);
    (value, captured)
}

pub(super) fn typed<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).expect("typed value")
}

pub(super) fn kwargs(extra: &str) -> MetadataMap {
    let mut base: MetadataMap = typed(BASE_KWARGS);
    base.extend(typed::<MetadataMap>(extra));
    base
}

pub(super) fn call_of<'call>(
    kwargs: &'call MetadataMap,
    response: Option<&'call OtelModelValue>,
) -> MetricCall<'call> {
    MetricCall {
        kwargs,
        response,
        start_time: Decimal::from(10_i64),
        end_time: Decimal::from(13_i64),
    }
}

pub(super) fn build(
    settings: Arc<dyn CallbackSettingsView>,
    callback_name: Option<&str>,
) -> (
    GenAiMetricRecorder,
    SdkMeterProvider,
    InMemoryMetricExporter,
) {
    build_in_offset(settings, callback_name, 0)
}

pub(super) fn build_in_offset(
    settings: Arc<dyn CallbackSettingsView>,
    callback_name: Option<&str>,
    offset_seconds: i32,
) -> (
    GenAiMetricRecorder,
    SdkMeterProvider,
    InMemoryMetricExporter,
) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    let recorder = GenAiMetricRecorder::new(
        create_instruments(&provider.meter("genai-metrics-test")),
        callback_name.map(str::to_owned),
        settings,
        FixedOffset::east_opt(offset_seconds).expect("offset"),
    )
    .expect("recorder");
    (recorder, provider, exporter)
}

pub(super) fn flush(
    provider: &SdkMeterProvider,
    exporter: &InMemoryMetricExporter,
) -> Vec<Recorded> {
    provider.force_flush().expect("flush");
    let finished = exporter.get_finished_metrics().expect("finished metrics");
    let mut recorded = Vec::new();
    for metric in finished
        .iter()
        .flat_map(ResourceMetrics::scope_metrics)
        .flat_map(ScopeMetrics::metrics)
    {
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            continue;
        };
        for point in histogram.data_points() {
            recorded.push(Recorded {
                name: metric.name().to_owned(),
                attributes: point
                    .attributes()
                    .map(|pair| (pair.key.to_string(), pair.value.to_string()))
                    .collect(),
                count: point.count(),
                sum: point.sum().to_string(),
            });
        }
    }
    recorded
}

pub(super) fn record_with(
    settings: Arc<dyn CallbackSettingsView>,
    callback_name: Option<&str>,
    extra: &str,
    response: Option<&str>,
) -> Vec<Recorded> {
    let (recorder, provider, exporter) = build(settings, callback_name);
    let call_kwargs = kwargs(extra);
    let typed_response: Option<OtelModelValue> = response.map(typed);
    recorder
        .record(&call_of(&call_kwargs, typed_response.as_ref()))
        .expect("record");
    flush(&provider, &exporter)
}

pub(super) fn record(extra: &str, response: Option<&str>) -> Vec<Recorded> {
    record_with(Arc::new(NoSettings), None, extra, response)
}

pub(super) fn record_result(
    extra: &str,
    response: Option<&str>,
) -> Result<Vec<Recorded>, LegacyMetricsError> {
    let (recorder, provider, exporter) = build(Arc::new(NoSettings), None);
    let call_kwargs = kwargs(extra);
    let typed_response: Option<OtelModelValue> = response.map(typed);
    recorder.record(&call_of(&call_kwargs, typed_response.as_ref()))?;
    Ok(flush(&provider, &exporter))
}

pub(super) fn fail_with(
    settings: Arc<dyn CallbackSettingsView>,
    callback_name: Option<&str>,
    extra: &str,
) -> Vec<Recorded> {
    let (recorder, provider, exporter) = build(settings, callback_name);
    recorder
        .record_failure(&kwargs(extra), Decimal::from(10_i64), Decimal::from(13_i64))
        .expect("record_failure");
    flush(&provider, &exporter)
}

pub(super) fn failure(extra: &str) -> Vec<Recorded> {
    fail_with(Arc::new(NoSettings), None, extra)
}

pub(super) fn sums(recorded: &[Recorded], name: &str) -> Vec<String> {
    let mut found: Vec<String> = recorded
        .iter()
        .filter(|entry| entry.name == name)
        .map(|entry| entry.sum.clone())
        .collect();
    found.sort();
    found
}

pub(super) fn attributes_of(recorded: &[Recorded], name: &str) -> BTreeMap<String, String> {
    recorded
        .iter()
        .find(|entry| entry.name == name)
        .expect("metric")
        .attributes
        .clone()
}

pub(super) fn names(recorded: &[Recorded]) -> Vec<&str> {
    let mut found: Vec<&str> = recorded.iter().map(|entry| entry.name.as_str()).collect();
    found.sort_unstable();
    found.dedup();
    found
}
