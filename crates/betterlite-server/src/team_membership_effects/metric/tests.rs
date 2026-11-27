#![cfg(test)]

use std::fs;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::team_rows::member::{Member, MemberBase, MemberRole};
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use rust_decimal::Decimal;
use tempfile::NamedTempFile;
use tracing::subscriber::{DefaultGuard, set_default};

use super::emit_team_members_metric;

const GAUGE: &str = "litellm_team_members_metric";
const FAILURE_PREFIX: &str = "Prometheus: failed to emit team members metric: ";

#[derive(Clone, Debug, Eq, PartialEq)]
struct GaugeEvent {
    labels: Vec<(String, String)>,
    update: MetricUpdate,
}

#[derive(Clone)]
struct RecordingGauge {
    labels: Vec<(String, String)>,
    events: Arc<Mutex<Vec<GaugeEvent>>>,
}

impl Collector for RecordingGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            labels: labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            events: Arc::clone(&self.events),
        }
    }

    fn update(&self, update: MetricUpdate) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GaugeEvent {
                labels: self.labels.clone(),
                update,
            });
    }
}

type EventLog = Arc<Mutex<Vec<GaugeEvent>>>;

fn recording_gauge(labels: &[&str]) -> (GaugeField<RecordingGauge>, EventLog) {
    let events: EventLog = Arc::default();
    let gauge = GaugeField {
        metric_name: GAUGE,
        supported_labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        collector: RecordingGauge {
            labels: Vec::new(),
            events: Arc::clone(&events),
        },
    };
    (gauge, events)
}

fn events(log: &EventLog) -> Vec<GaugeEvent> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn values_source(settings: &PrometheusLabelSettings) -> LabelSource<'_> {
    LabelSource::Values {
        settings,
        end_user_settings: EndUserCostTrackingSettings::default(),
    }
}

fn member(user_id: &str, email: &str, role: MemberRole) -> Member {
    Member {
        base: MemberBase {
            user_id: Some(user_id.to_owned()),
            user_email: Some(email.to_owned()),
        },
        role,
    }
}

fn roster() -> Vec<Member> {
    vec![
        member("test-user-1", "test-1@example.com", MemberRole::Admin),
        member("test-user-2", "test-2@example.com", MemberRole::User),
    ]
}

fn set_event(labels: &[(&str, &str)], count: u64) -> GaugeEvent {
    GaugeEvent {
        labels: labels
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        update: MetricUpdate::Set(MetricValue::Finite(Decimal::from(count))),
    }
}

fn capture_log() -> (DefaultGuard, NamedTempFile) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (set_default(subscriber), log)
}

fn logged_text(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).unwrap()
}

#[test]
fn an_absent_gauge_records_and_logs_nothing() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (guard, log) = capture_log();
    emit_team_members_metric::<RecordingGauge>(
        None,
        values_source(&settings),
        "test-team-1",
        Some("Test Team"),
        &roster(),
    );
    drop(guard);
    assert!(logged_text(&log).is_empty());
}

#[test]
fn the_gauge_counts_the_roster_with_the_team_labels() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauge, log) = recording_gauge(&["team", "team_alias"]);
    emit_team_members_metric(
        Some(&gauge),
        values_source(&settings),
        "test-team-1",
        Some("Test Team"),
        &roster(),
    );
    assert_eq!(
        events(&log),
        vec![set_event(
            &[("team", "test-team-1"), ("team_alias", "Test Team")],
            2,
        )]
    );
}

#[test]
fn an_absent_alias_is_an_empty_alias_label() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauge, log) = recording_gauge(&["team", "team_alias"]);
    emit_team_members_metric(
        Some(&gauge),
        values_source(&settings),
        "test-team-1",
        None,
        &roster(),
    );
    assert_eq!(
        events(&log),
        vec![set_event(&[("team", "test-team-1"), ("team_alias", "")], 2,)]
    );
}

#[test]
fn an_empty_roster_sets_zero() {
    let settings = PrometheusLabelSettings::with_defaults();
    let (gauge, log) = recording_gauge(&["team", "team_alias"]);
    emit_team_members_metric(
        Some(&gauge),
        values_source(&settings),
        "test-team-1",
        Some("Test Team"),
        &[],
    );
    assert_eq!(
        events(&log),
        vec![set_event(
            &[("team", "test-team-1"), ("team_alias", "Test Team")],
            0,
        )]
    );
}

#[test]
fn a_label_failure_is_logged_at_debug_and_swallowed() {
    let settings = PrometheusLabelSettings::with_defaults();
    let context = PrometheusLabelFactoryContext::new(
        &UserApiKeyLabelValues::from_entries(&[(
            "team".to_owned(),
            LabelDumpValue::String("test-team-other".to_owned()),
        )]),
        &settings,
        EndUserCostTrackingSettings::default(),
    )
    .expect("the label context builds");
    let (gauge, log) = recording_gauge(&["team", "team_alias"]);
    let (guard, captured) = capture_log();
    emit_team_members_metric(
        Some(&gauge),
        LabelSource::Context(&context),
        "test-team-1",
        Some("Test Team"),
        &roster(),
    );
    drop(guard);
    assert!(events(&log).is_empty());
    let text = logged_text(&captured);
    let failure = format!("{FAILURE_PREFIX}the label context was built from different enum values");
    assert!(text.contains(failure.as_str()), "{text}");
}
