use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::bounded_series::CounterEmission;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::BoundedPrometheusSeriesTracker;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prometheus::collector_factory::collector::MetricValue;
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;

use limits::{series_limits, time_at};
use output::events_value;
use probe::{ProbeLog, ProbeMetric, probe_metrics};

mod limits;
mod output;
mod probe;
mod tests;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    TrackEndUser(TrackScenario),
    IncLabeledCounter(EmissionScenario),
}

#[derive(serde::Deserialize)]
struct TrackScenario {
    metric_name: String,
    supported_labels: Vec<String>,
    metrics: BTreeMap<String, Outcome>,
    steps: Vec<TrackStep>,
}

#[derive(serde::Deserialize)]
struct TrackStep {
    metric: String,
    labels: BTreeMap<String, Option<String>>,
    at_seconds: f64,
    max_series: Option<i64>,
    ttl_seconds: Option<f64>,
    cleanup_interval_seconds: Option<f64>,
}

#[derive(serde::Deserialize)]
struct EmissionScenario {
    metric_name: String,
    supported_labels: Vec<String>,
    enable_end_user_tracking: bool,
    max_series: Option<i64>,
    ttl_seconds: Option<f64>,
    cleanup_interval_seconds: Option<f64>,
    amount: u64,
    emissions: Vec<EmissionStep>,
}

#[derive(serde::Deserialize)]
struct EmissionStep {
    values: BTreeMap<String, Option<String>>,
    at_seconds: f64,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Removed,
    Absent,
    Unsupported,
    Failed,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::TrackEndUser(scenario) => track_end_user(scenario),
            Input::IncLabeledCounter(scenario) => inc_labeled_counter(scenario),
        }
    }
}

fn track_end_user(scenario: &TrackScenario) -> HelperResult {
    let log = Arc::new(Mutex::new(ProbeLog::default()));
    let metrics = probe_metrics(scenario, &log);
    let tracker = BoundedPrometheusSeriesTracker::default();
    let base = Instant::now();
    for step in &scenario.steps {
        let limits = series_limits(
            step.max_series,
            step.ttl_seconds,
            step.cleanup_interval_seconds,
        )?;
        let counter = metrics
            .get(&step.metric)
            .ok_or_else(|| invalid(&format!("no metric named {}", step.metric)))?;
        let now = time_at(base, step.at_seconds)?;
        let emission = CounterEmission {
            counter,
            metric_name: &scenario.metric_name,
            supported_labels: &scenario.supported_labels,
            tracker: &tracker,
            limits: &limits,
        };
        emission
            .track_end_user(&step.labels, now)
            .map_err(|error| invalid(&error.to_string()))?;
    }
    Ok(events_value(&log))
}

fn inc_labeled_counter(scenario: &EmissionScenario) -> HelperResult {
    let log = Arc::new(Mutex::new(ProbeLog::default()));
    let limits = series_limits(
        scenario.max_series,
        scenario.ttl_seconds,
        scenario.cleanup_interval_seconds,
    )?;
    let counter = ProbeMetric {
        name: scenario.metric_name.clone(),
        outcome: Outcome::Removed,
        labels: Vec::new(),
        log: Arc::clone(&log),
    };
    let settings = PrometheusLabelSettings::with_defaults();
    let source = LabelSource::Values {
        settings: &settings,
        end_user_settings: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: None,
            enable_prometheus_only: Some(scenario.enable_end_user_tracking),
        },
    };
    let tracker = BoundedPrometheusSeriesTracker::default();
    let emission = CounterEmission {
        counter: &counter,
        metric_name: &scenario.metric_name,
        supported_labels: &scenario.supported_labels,
        tracker: &tracker,
        limits: &limits,
    };
    let amount = MetricValue::Finite(Decimal::from(scenario.amount));
    let base = Instant::now();
    for step in &scenario.emissions {
        let values = label_values(&step.values);
        let now = time_at(base, step.at_seconds)?;
        emission
            .inc_labeled_counter(&values, source, amount, now)
            .map_err(|error| invalid(&error.to_string()))?;
    }
    Ok(events_value(&log))
}

fn label_values(values: &BTreeMap<String, Option<String>>) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = values
        .iter()
        .map(|(name, value)| {
            let dumped = value.as_ref().map_or(LabelDumpValue::Null, |text| {
                LabelDumpValue::String(text.clone())
            });
            (name.clone(), dumped)
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
