#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::model_visibility::Settings as ListingSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cooldown_decisions::failure::{ErrorStatus, FailureClass, Seconds};
use betterlite_router::cooldown_state::health::{
    DeploymentHealthCache, HealthSnapshot, HealthSnapshotError,
};
use betterlite_router::cooldown_state::settings::Settings as RouterSettings;
use betterlite_router::cooldown_state::shared::{HealthReadFuture, SharedHealth};
use betterlite_router::router_observations::counter::{CounterError, CounterSink, Increment};
use betterlite_router::router_observations::failure::{
    CooldownError, CooldownSink, FailureError, FailureObservation,
};
use betterlite_router::routing_value_store::Clock;
use serde_json::Number;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::health_cycle::ports::ProbeFailure;

use super::{HealthStateError, RouterState, exception_status, write_health_state};

struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    fn at(seconds: f64) -> Self {
        Self(Arc::new(Mutex::new(seconds)))
    }

    fn handle(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[derive(Default)]
struct RecordingHealth {
    current: Mutex<Option<HealthSnapshot>>,
    writes: Mutex<Vec<(HealthSnapshot, i64)>>,
}

impl RecordingHealth {
    fn stored(&self) -> Option<HealthSnapshot> {
        self.current
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn writes(&self) -> Vec<(HealthSnapshot, i64)> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SharedHealth for RecordingHealth {
    fn read(&self) -> HealthReadFuture<'_> {
        Box::pin(async move { self.stored() })
    }

    fn read_raw(&self) -> Option<HealthSnapshot> {
        self.stored()
    }

    fn write(&self, snapshot: &HealthSnapshot, ttl_seconds: i64) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((snapshot.clone(), ttl_seconds));
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot.clone());
    }
}

#[derive(Default)]
struct RecordingCounters {
    values: Mutex<BTreeMap<String, i64>>,
    increments: Mutex<Vec<(String, i64, Duration)>>,
    reject: Option<String>,
}

impl RecordingCounters {
    fn rejecting(key: &str) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            increments: Mutex::new(Vec::new()),
            reject: Some(key.to_owned()),
        }
    }

    fn increments(&self) -> Vec<(String, i64, Duration)> {
        self.increments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn value(&self, key: &str) -> Option<i64> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }
}

impl CounterSink for RecordingCounters {
    fn increment(&self, key: &str, value: i64, ttl: Duration) -> Result<(), CounterError> {
        if self.reject.as_deref() == Some(key) {
            return Err(CounterError::Rejected {
                key: key.to_owned(),
            });
        }
        self.increments
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl));
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let stored = values.entry(key.to_owned()).or_insert(0);
        *stored = stored.saturating_add(value);
        drop(values);
        Ok(())
    }

    fn increment_pipeline(&self, increments: &[Increment]) -> Result<(), CounterError> {
        for increment in increments {
            self.increment(&increment.key, increment.value, increment.ttl)?;
        }
        Ok(())
    }

    fn read(&self, key: &str) -> Option<i64> {
        self.value(key)
    }

    fn write(&self, key: &str, value: i64, _ttl: Option<Duration>) {
        let _previous = self
            .values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedCooldown {
    deployment_id: String,
    class: FailureClass,
    status: ErrorStatus,
    requested_model_group: Option<String>,
    time_to_cooldown: Seconds,
    successes: u64,
    failures: u64,
}

#[derive(Default)]
struct RecordingCooldowns {
    recorded: Mutex<Vec<RecordedCooldown>>,
}

impl RecordingCooldowns {
    fn recorded(&self) -> Vec<RecordedCooldown> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CooldownSink for RecordingCooldowns {
    fn set_cooldown_deployments(
        &self,
        observation: &FailureObservation<'_>,
    ) -> Result<bool, CooldownError> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCooldown {
                deployment_id: observation.deployment_id.to_owned(),
                class: observation.failure.class,
                status: observation.failure.status.clone(),
                requested_model_group: observation.requested_model_group.map(str::to_owned),
                time_to_cooldown: observation.time_to_cooldown,
                successes: observation.minute.successes,
                failures: observation.minute.failures,
            });
        Ok(false)
    }
}

struct Fixture {
    clock: TestClock,
    health: Arc<RecordingHealth>,
    counters: RecordingCounters,
    cooldowns: RecordingCooldowns,
    states: DeploymentHealthCache,
    settings: RouterSettings,
}

impl Fixture {
    fn new(settings: RouterSettings) -> Self {
        Self::with_staleness(settings, 600.0)
    }

    fn with_staleness(settings: RouterSettings, staleness: f64) -> Self {
        let clock = TestClock::at(100.0);
        let health = Arc::new(RecordingHealth::default());
        let shared = Arc::clone(&health);
        let states = DeploymentHealthCache::new(staleness, shared, clock.handle());
        Self {
            clock,
            health,
            counters: RecordingCounters::default(),
            cooldowns: RecordingCooldowns::default(),
            states,
            settings,
        }
    }

    fn with_counters(settings: RouterSettings, counters: RecordingCounters) -> Self {
        let mut fixture = Self::new(settings);
        fixture.counters = counters;
        fixture
    }

    fn router(&self) -> RouterState<'_> {
        RouterState {
            settings: &self.settings,
            health_states: &self.states,
            counters: &self.counters,
            cooldowns: &self.cooldowns,
            clock: self.clock.handle(),
        }
    }
}

fn settings(routing: bool) -> RouterSettings {
    RouterSettings {
        default_cooldown_time: 12.5,
        redis_read_interval_seconds: 1.0,
        enable_health_check_routing: routing,
        staleness_threshold: 600.0,
        ignore_transient_errors: false,
    }
}

fn ignoring_settings(routing: bool) -> RouterSettings {
    RouterSettings {
        ignore_transient_errors: true,
        ..settings(routing)
    }
}

fn listing(enabled: bool) -> ListingSettings {
    ListingSettings {
        model_list_healthy_only: enabled,
    }
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn failure(status: u16) -> ProbeFailure {
    ProbeFailure {
        message: format!("the probe failed with {status}"),
        status_code: status,
        raw_request_typed_dict: None,
    }
}

fn exceptions(entries: &[(&str, u16)]) -> BTreeMap<String, ProbeFailure> {
    entries
        .iter()
        .map(|(model_id, status)| ((*model_id).to_owned(), failure(*status)))
        .collect()
}

fn write_state(
    fixture: &Fixture,
    listing_settings: ListingSettings,
    healthy: &[JsonObject],
    unhealthy: &[JsonObject],
    live: &BTreeMap<String, ProbeFailure>,
) {
    let router = fixture.router();
    write_health_state(Some(&router), listing_settings, healthy, unhealthy, live);
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn text(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).expect("utf-8 log output")
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[test]
fn no_router_writes_nothing() {
    let (logs, _guard) = capture_logs();
    let healthy = [object(vec![("model_id", text("a"))])];
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let live = exceptions(&[("b", 500)]);
    write_health_state(None, listing(true), &healthy, &unhealthy, &live);
    assert!(logs.text().is_empty());
}

#[test]
fn both_switches_off_write_nothing() {
    let fixture = Fixture::new(settings(false));
    let healthy = [object(vec![("model_id", text("a"))])];
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let live = exceptions(&[("b", 500)]);
    write_state(&fixture, listing(false), &healthy, &unhealthy, &live);
    assert!(fixture.health.writes().is_empty());
    assert!(fixture.counters.increments().is_empty());
    assert!(fixture.cooldowns.recorded().is_empty());
}

#[test]
fn routing_alone_publishes_the_states_and_counts_the_failure() {
    let fixture = Fixture::new(settings(true));
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let live = exceptions(&[("b", 503)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    let written = fixture.health.writes();
    assert_eq!(written.len(), 1);
    let state = written[0].0.get("b").expect("b is in the published state");
    assert!(!state.is_healthy);
    assert_eq!(fixture.counters.value("b:fails"), Some(1));
}

#[test]
fn listing_alone_publishes_the_states_without_counting() {
    let fixture = Fixture::new(settings(false));
    let unhealthy = [object(vec![("model_id", text("b"))])];
    let live = exceptions(&[("b", 500)]);
    write_state(&fixture, listing(true), &[], &unhealthy, &live);
    assert_eq!(fixture.health.writes().len(), 1);
    assert!(fixture.counters.increments().is_empty());
    assert!(fixture.cooldowns.recorded().is_empty());
}

#[test]
fn the_published_states_carry_each_verdict_and_the_clock_timestamp() {
    let fixture = Fixture::new(settings(false));
    let healthy = [object(vec![("model_id", text("ok"))])];
    let unhealthy = [object(vec![
        ("model_id", text("down")),
        ("exception_status", number(500)),
    ])];
    let live = exceptions(&[("down", 500)]);
    write_state(&fixture, listing(true), &healthy, &unhealthy, &live);
    let snapshot = fixture.health.stored().expect("one snapshot was published");
    let up = snapshot.get("ok").expect("the healthy row keeps its id");
    assert!(up.is_healthy);
    assert_eq!(up.timestamp.to_bits(), 100.0_f64.to_bits());
    assert_eq!(up.reason, "");
    let down = snapshot
        .get("down")
        .expect("the unhealthy row keeps its id");
    assert!(!down.is_healthy);
    assert_eq!(down.reason, "background_health_check_failed");
}

#[test]
fn ignored_transient_failures_stay_out_of_the_published_states() {
    let fixture = Fixture::new(ignoring_settings(false));
    let unhealthy = [
        object(vec![
            ("model_id", text("live429")),
            ("exception_status", number(429)),
        ]),
        object(vec![
            ("model_id", text("live408")),
            ("exception_status", number(408)),
        ]),
        object(vec![
            ("model_id", text("stored429")),
            ("exception_status", number(429)),
        ]),
        object(vec![
            ("model_id", text("live500")),
            ("exception_status", number(500)),
        ]),
        // the live status wins over the stored one, both ways round
        object(vec![
            ("model_id", text("mismatch")),
            ("exception_status", number(500)),
        ]),
        object(vec![
            ("model_id", text("kept")),
            ("exception_status", number(429)),
        ]),
    ];
    let live = exceptions(&[
        ("live429", 429),
        ("live408", 408),
        ("live500", 500),
        ("mismatch", 429),
        ("kept", 500),
    ]);
    write_state(&fixture, listing(true), &[], &unhealthy, &live);
    let snapshot = fixture.health.stored().expect("the rest are published");
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.contains_key("live500"));
    assert!(snapshot.contains_key("kept"));
}

#[test]
fn transient_failures_stay_unhealthy_without_the_ignore_flag() {
    let fixture = Fixture::new(settings(false));
    let unhealthy = [object(vec![
        ("model_id", text("limited")),
        ("exception_status", number(429)),
    ])];
    let live = exceptions(&[("limited", 429)]);
    write_state(&fixture, listing(true), &[], &unhealthy, &live);
    let snapshot = fixture.health.stored().expect("the row is published");
    let state = snapshot.get("limited").expect("the row keeps its verdict");
    assert!(!state.is_healthy);
}

#[test]
fn rows_without_an_id_publish_nothing() {
    let fixture = Fixture::new(settings(false));
    let (logs, _guard) = capture_logs();
    let unhealthy = [object(vec![("model", text("openai/b"))])];
    write_state(&fixture, listing(true), &[], &unhealthy, &BTreeMap::new());
    assert!(fixture.health.writes().is_empty());
    assert!(!logs.text().contains("health_check_routing_state_updated"));
}

#[test]
fn the_debug_line_reports_each_verdict_count() {
    let fixture = Fixture::new(settings(false));
    let (logs, _guard) = capture_logs();
    let healthy = [object(vec![("model_id", text("up"))])];
    let unhealthy = [
        object(vec![("model_id", text("down-a"))]),
        object(vec![("model_id", text("down-b"))]),
    ];
    write_state(
        &fixture,
        listing(true),
        &healthy,
        &unhealthy,
        &BTreeMap::new(),
    );
    assert!(
        logs.text()
            .contains("health_check_routing_state_updated healthy=1 unhealthy=2")
    );
}

#[test]
fn a_live_failure_counts_a_failure_and_asks_for_a_cooldown() {
    let fixture = Fixture::new(settings(true));
    fixture.counters.write("dep:successes", 3, None);
    let unhealthy = [object(vec![
        ("model_id", text("dep")),
        ("exception_status", number(500)),
    ])];
    let live = exceptions(&[("dep", 500)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    assert_eq!(fixture.counters.value("dep:fails"), Some(1));
    assert_eq!(
        fixture.counters.increments(),
        vec![("dep:fails".to_owned(), 1, Duration::from_secs(60))]
    );
    assert_eq!(
        fixture.cooldowns.recorded(),
        vec![RecordedCooldown {
            deployment_id: "dep".to_owned(),
            class: FailureClass::Unclassified,
            status: ErrorStatus::Code(500),
            requested_model_group: None,
            time_to_cooldown: Seconds(12.5),
            successes: 3,
            failures: 1,
        }]
    );
}

#[test]
fn the_failure_loop_needs_an_id_and_a_live_exception() {
    let fixture = Fixture::new(settings(true));
    let unhealthy = [
        object(vec![
            ("model_id", text("counted")),
            ("exception_status", number(500)),
        ]),
        object(vec![
            ("model_id", text("stored")),
            ("exception_status", number(500)),
        ]),
        object(vec![("model", text("openai/no-id"))]),
    ];
    let live = exceptions(&[("counted", 500)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    assert_eq!(
        fixture.counters.increments(),
        vec![("counted:fails".to_owned(), 1, Duration::from_secs(60))]
    );
    let recorded = fixture.cooldowns.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].deployment_id, "counted");
}

#[test]
fn ignored_transients_are_not_counted() {
    let fixture = Fixture::new(ignoring_settings(true));
    let unhealthy = [
        object(vec![
            ("model_id", text("limited")),
            ("exception_status", number(429)),
        ]),
        object(vec![
            ("model_id", text("timeout")),
            ("exception_status", number(408)),
        ]),
        object(vec![
            ("model_id", text("broken")),
            ("exception_status", number(500)),
        ]),
    ];
    let live = exceptions(&[("limited", 429), ("timeout", 408), ("broken", 500)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    assert_eq!(
        fixture.counters.increments(),
        vec![("broken:fails".to_owned(), 1, Duration::from_secs(60))]
    );
    let recorded = fixture.cooldowns.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].deployment_id, "broken");
}

#[test]
fn transient_failures_are_counted_without_the_ignore_flag() {
    let fixture = Fixture::new(settings(true));
    let unhealthy = [object(vec![
        ("model_id", text("limited")),
        ("exception_status", number(429)),
    ])];
    let live = exceptions(&[("limited", 429)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    assert_eq!(fixture.counters.value("limited:fails"), Some(1));
    let recorded = fixture.cooldowns.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].status, ErrorStatus::Code(429));
}

#[test]
fn a_count_failure_logs_the_warning_and_skips_the_rest() {
    let fixture =
        Fixture::with_counters(settings(true), RecordingCounters::rejecting("first:fails"));
    let (logs, _guard) = capture_logs();
    let unhealthy = [
        object(vec![
            ("model_id", text("first")),
            ("exception_status", number(500)),
        ]),
        object(vec![
            ("model_id", text("second")),
            ("exception_status", number(500)),
        ]),
    ];
    let live = exceptions(&[("first", 500), ("second", 500)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    let expected = HealthStateError::Failure {
        deployment_id: "first".to_owned(),
        source: Box::new(FailureError::Counter(CounterError::Rejected {
            key: "first:fails".to_owned(),
        })),
    };
    assert!(logs.text().contains(&format!(
        "Failed to write health state to router cache: {expected}"
    )));
    assert!(fixture.cooldowns.recorded().is_empty());
    assert_eq!(fixture.counters.value("second:fails"), None);
}

#[test]
fn a_state_failure_logs_the_warning_and_skips_the_failure_loop() {
    let fixture = Fixture::with_staleness(settings(true), f64::NAN);
    let (logs, _guard) = capture_logs();
    let unhealthy = [object(vec![
        ("model_id", text("dep")),
        ("exception_status", number(500)),
    ])];
    let live = exceptions(&[("dep", 500)]);
    write_state(&fixture, listing(false), &[], &unhealthy, &live);
    let expected = HealthStateError::States(HealthSnapshotError::Expiry { seconds: f64::NAN });
    assert!(logs.text().contains(&format!(
        "Failed to write health state to router cache: {expected}"
    )));
    assert!(fixture.counters.increments().is_empty());
}

#[test]
fn the_live_status_answers_before_the_stored_one() {
    let endpoint = object(vec![
        ("model_id", text("dep")),
        ("exception_status", number(503)),
    ]);
    let live = exceptions(&[("dep", 429)]);
    assert_eq!(exception_status(&endpoint, &live), 429);
}

#[test]
fn the_stored_status_answers_without_a_live_exception() {
    let endpoint = object(vec![
        ("model_id", text("dep")),
        ("exception_status", number(408)),
    ]);
    assert_eq!(exception_status(&endpoint, &BTreeMap::new()), 408);
}

#[test]
fn a_row_with_no_status_answers_500() {
    let endpoint = object(vec![("model_id", text("dep"))]);
    assert_eq!(exception_status(&endpoint, &BTreeMap::new()), 500);
}

#[test]
fn a_stored_status_that_is_not_a_whole_number_answers_500() {
    let text_status = object(vec![
        ("model_id", text("dep")),
        ("exception_status", text("429")),
    ]);
    assert_eq!(exception_status(&text_status, &BTreeMap::new()), 500);
    let null_status = object(vec![
        ("model_id", text("dep")),
        ("exception_status", Json::Null),
    ]);
    assert_eq!(exception_status(&null_status, &BTreeMap::new()), 500);
    let fraction = Number::from_f64(429.5).expect("429.5 is a number");
    let fractional = object(vec![
        ("model_id", text("dep")),
        ("exception_status", Json::Number(fraction)),
    ]);
    assert_eq!(exception_status(&fractional, &BTreeMap::new()), 500);
    let too_big = object(vec![
        ("model_id", text("dep")),
        ("exception_status", number(70_000)),
    ]);
    assert_eq!(exception_status(&too_big, &BTreeMap::new()), 500);
}

#[test]
fn a_row_without_an_id_uses_its_stored_status() {
    let endpoint = object(vec![
        ("model", text("openai/x")),
        ("exception_status", number(408)),
    ]);
    let live = exceptions(&[("", 429)]);
    assert_eq!(exception_status(&endpoint, &live), 408);
}
