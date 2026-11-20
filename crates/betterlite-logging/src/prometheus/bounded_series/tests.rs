#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_config::sections::prometheus_bounded_series::Settings as BoundedSeriesSettings;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;

use super::{CounterEmission, SeriesLimits};
use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_helpers::bounded_series_tracker::{
    BoundedPrometheusSeriesTracker, ChildRemoval, RemovableMetric,
};
use crate::prometheus_label_projection::prometheus_label_factory::LabelSource;
use crate::prometheus_label_schema::PrometheusLabelSettings;

const METRIC: &str = "litellm_requests_metric";
const ONE: MetricValue = MetricValue::Finite(Decimal::ONE);

#[derive(Debug, thiserror::Error)]
#[error("the probe counter refused the removal")]
struct ProbeError;

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProbeEvent {
    Increment {
        labels: Vec<(String, String)>,
        amount: MetricValue,
    },
    Removal {
        labels: Vec<Option<String>>,
    },
}

#[derive(Clone, Debug, Default)]
struct ProbeCounter {
    labels: Vec<(String, String)>,
    log: Arc<Mutex<Vec<ProbeEvent>>>,
}

impl Collector for ProbeCounter {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        let mut child = self.clone();
        child.labels = labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        child
    }

    fn update(&self, update: MetricUpdate) {
        if let MetricUpdate::Inc(value) = update {
            self.log
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(ProbeEvent::Increment {
                    labels: self.labels.clone(),
                    amount: value,
                });
        }
    }
}

impl RemovableMetric for ProbeCounter {
    type Error = ProbeError;

    fn remove_child(&self, label_values: &[Option<String>]) -> Result<ChildRemoval, Self::Error> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ProbeEvent::Removal {
                labels: label_values.to_vec(),
            });
        Ok(ChildRemoval::Removed)
    }
}

struct EmissionFixture {
    counter: ProbeCounter,
    tracker: BoundedPrometheusSeriesTracker,
    supported: Vec<String>,
    settings: PrometheusLabelSettings,
    limits: SeriesLimits,
}

impl EmissionFixture {
    fn new(supported: &[&str], limits: SeriesLimits) -> Self {
        Self {
            counter: ProbeCounter::default(),
            tracker: BoundedPrometheusSeriesTracker::default(),
            supported: words(supported),
            settings: PrometheusLabelSettings::with_defaults(),
            limits,
        }
    }

    fn emission(&self) -> CounterEmission<'_, ProbeCounter> {
        CounterEmission {
            counter: &self.counter,
            metric_name: METRIC,
            supported_labels: &self.supported,
            tracker: &self.tracker,
            limits: &self.limits,
        }
    }

    fn source(&self, end_user_settings: EndUserCostTrackingSettings) -> LabelSource<'_> {
        LabelSource::Values {
            settings: &self.settings,
            end_user_settings,
        }
    }
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().copied().map(str::to_owned).collect()
}

fn values(pairs: &[(&str, Option<&str>)]) -> UserApiKeyLabelValues {
    let entries: Vec<(String, LabelDumpValue)> = pairs
        .iter()
        .map(|(name, value)| {
            let dumped = value.map_or(LabelDumpValue::Null, |text| {
                LabelDumpValue::String(text.to_owned())
            });
            ((*name).to_owned(), dumped)
        })
        .collect();
    UserApiKeyLabelValues::from_entries(&entries)
}

fn events(counter: &ProbeCounter) -> Vec<ProbeEvent> {
    counter
        .log
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn tracking_enabled() -> EndUserCostTrackingSettings {
    EndUserCostTrackingSettings {
        disable_end_user_cost_tracking: None,
        enable_prometheus_only: Some(true),
    }
}

fn increment(labels: &[(&str, &str)], amount: MetricValue) -> ProbeEvent {
    ProbeEvent::Increment {
        labels: labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        amount,
    }
}

fn removal(labels: &[&str]) -> ProbeEvent {
    ProbeEvent::Removal {
        labels: labels
            .iter()
            .map(|value| Some((*value).to_owned()))
            .collect(),
    }
}

#[test]
fn increments_with_the_projected_labels() {
    let fixture = EmissionFixture::new(&["model", "user"], SeriesLimits::default());
    let labelled = values(&[("model", Some("gpt-4o")), ("user", Some("alice"))]);
    let amount = MetricValue::Finite(Decimal::from(2_u32));
    fixture
        .emission()
        .inc_labeled_counter(
            &labelled,
            fixture.source(EndUserCostTrackingSettings::default()),
            amount,
            Instant::now(),
        )
        .unwrap();
    assert_eq!(
        events(&fixture.counter),
        vec![increment(&[("model", "gpt-4o"), ("user", "alice")], amount)]
    );
}

#[test]
fn increments_before_tracking_the_series() {
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["model", "end_user"], limits);
    let start = Instant::now();
    for (offset, end_user) in [(0_u64, "e1"), (1, "e2"), (2, "e3")] {
        let labelled = values(&[("model", Some("m")), ("end_user", Some(end_user))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                start + Duration::from_secs(offset),
            )
            .unwrap();
    }
    assert_eq!(
        events(&fixture.counter),
        vec![
            increment(&[("model", "m"), ("end_user", "e1")], ONE),
            increment(&[("model", "m"), ("end_user", "e2")], ONE),
            removal(&["m", "e1"]),
            increment(&[("model", "m"), ("end_user", "e3")], ONE),
            removal(&["m", "e2"]),
        ]
    );
}

#[test]
fn tracks_a_metric_whose_only_label_is_the_end_user() {
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["end_user"], limits);
    let start = Instant::now();
    for (offset, end_user) in [(0_u64, "e1"), (1, "e2")] {
        let labelled = values(&[("end_user", Some(end_user))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                start + Duration::from_secs(offset),
            )
            .unwrap();
    }
    assert_eq!(
        events(&fixture.counter),
        vec![
            increment(&[("end_user", "e1")], ONE),
            increment(&[("end_user", "e2")], ONE),
            removal(&["e1"]),
        ]
    );
}

#[test]
fn tracks_the_series_in_the_supported_label_order() {
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["end_user", "model"], limits);
    let start = Instant::now();
    for (offset, end_user) in [(0_u64, "e1"), (1, "e2")] {
        let labelled = values(&[("model", Some("m")), ("end_user", Some(end_user))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                start + Duration::from_secs(offset),
            )
            .unwrap();
    }
    let removals: Vec<ProbeEvent> = events(&fixture.counter)
        .into_iter()
        .filter(|event| matches!(event, ProbeEvent::Removal { .. }))
        .collect();
    assert_eq!(removals, vec![removal(&["e1", "m"])]);
}

#[test]
fn renders_an_absent_label_value_as_none() {
    let fixture = EmissionFixture::new(&["model", "user"], SeriesLimits::default());
    let labelled = values(&[("model", Some("gpt-4o"))]);
    fixture
        .emission()
        .inc_labeled_counter(
            &labelled,
            fixture.source(EndUserCostTrackingSettings::default()),
            ONE,
            Instant::now(),
        )
        .unwrap();
    assert_eq!(
        events(&fixture.counter),
        vec![increment(&[("model", "gpt-4o"), ("user", "None")], ONE)]
    );
}

#[test]
fn skips_tracking_for_a_metric_without_the_end_user_label() {
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["model", "user"], limits);
    for user in ["u1", "u2", "u3"] {
        let labelled = values(&[("model", Some("m")), ("user", Some(user))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                Instant::now(),
            )
            .unwrap();
    }
    assert!(
        events(&fixture.counter)
            .iter()
            .all(|event| matches!(event, ProbeEvent::Increment { .. }))
    );
}

#[test]
fn skips_tracking_when_the_end_user_is_none() {
    let limits = SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["model", "end_user"], limits);
    for _round in 0..3 {
        let labelled = values(&[("model", Some("m"))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                Instant::now(),
            )
            .unwrap();
    }
    assert!(
        events(&fixture.counter)
            .iter()
            .all(|event| matches!(event, ProbeEvent::Increment { .. }))
    );
}

#[test]
fn skips_tracking_when_no_limit_is_configured() {
    let fixture = EmissionFixture::new(&["model", "end_user"], SeriesLimits::default());
    for end_user in ["e1", "e2", "e3"] {
        let labelled = values(&[("model", Some("m")), ("end_user", Some(end_user))]);
        fixture
            .emission()
            .inc_labeled_counter(
                &labelled,
                fixture.source(tracking_enabled()),
                ONE,
                Instant::now(),
            )
            .unwrap();
    }
    assert!(
        events(&fixture.counter)
            .iter()
            .all(|event| matches!(event, ProbeEvent::Increment { .. }))
    );
}

#[test]
fn tracks_a_series_when_only_the_ttl_is_configured() {
    let limits = SeriesLimits {
        max_series: None,
        ttl: Some(Duration::from_secs(5)),
        cleanup_interval: None,
    };
    let fixture = EmissionFixture::new(&["model", "end_user"], limits);
    let start = Instant::now();
    let first = values(&[("model", Some("m")), ("end_user", Some("e1"))]);
    fixture
        .emission()
        .inc_labeled_counter(&first, fixture.source(tracking_enabled()), ONE, start)
        .unwrap();
    let second = values(&[("model", Some("m")), ("end_user", Some("e2"))]);
    fixture
        .emission()
        .inc_labeled_counter(
            &second,
            fixture.source(tracking_enabled()),
            ONE,
            start + Duration::from_secs(10),
        )
        .unwrap();
    let removals: Vec<ProbeEvent> = events(&fixture.counter)
        .into_iter()
        .filter(|event| matches!(event, ProbeEvent::Removal { .. }))
        .collect();
    assert_eq!(removals, vec![removal(&["m", "e1"])]);
}

#[test]
fn builds_the_limits_from_the_config_section() {
    let settings = BoundedSeriesSettings {
        max_series: Some(5),
        ttl: Some(Duration::from_secs(10)),
        cleanup_interval: None,
    };
    assert_eq!(
        SeriesLimits::from(&settings),
        SeriesLimits {
            max_series: Some(5),
            ttl: Some(Duration::from_secs(10)),
            cleanup_interval: None,
        }
    );
}
