#![cfg(test)]

mod plans;
mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinSet;
use tracing::Level;
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use crate::reload::{FetchLimits, ReloadOutcome, RetryOutcome, StartupOutcome};
use crate::source::CatalogSnapshot;

use plans::Plan;

use support::{
    BlockingSleeper, CapturedLog, RecordingRegistrations, RecordingSleeper, Reply, ScriptedClient,
    ScriptedJitter, StopFlag, TestService, URL, blob, body_with_entry, bundled_service,
    catalog_body, limits, local_service, registrations_service, wide_bundled_service,
};

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

pub(super) fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn catalog_prefix<'revision>(
    revision: Option<&str>,
    alpha: &'revision str,
    beta: &'revision str,
) -> &'revision str {
    match revision {
        Some(value) if value == alpha => "alpha",
        Some(value) if value == beta => "beta",
        None => "",
        Some(other) => panic!("unexpected revision {other}"),
    }
}

pub(super) async fn start(
    service: &TestService,
    client: Arc<ScriptedClient>,
) -> StartupOutcome<ScriptedClient, RecordingSleeper, ScriptedJitter> {
    service
        .startup(
            limits(3),
            client,
            Arc::new(RecordingSleeper::default()),
            ScriptedJitter::new(&[]),
        )
        .await
}

pub(super) async fn reload_once(
    service: &TestService,
    client: &ScriptedClient,
    sleeper: &RecordingSleeper,
) -> ReloadOutcome {
    service
        .reload(limits(1), client, sleeper, &mut ScriptedJitter::new(&[]))
        .await
}

pub(super) fn adopted(outcome: ReloadOutcome) -> Arc<CatalogSnapshot> {
    match outcome {
        ReloadOutcome::Reloaded(snapshot) => snapshot,
        ReloadOutcome::Unavailable(unavailable) => {
            panic!("the reload was unavailable: {}", unavailable.reason);
        }
    }
}

#[rstest]
#[case::startup_local(Plan::StartupLocal)]
#[case::startup_remote(Plan::StartupRemote)]
#[case::startup_fallback(Plan::StartupFallback)]
#[case::startup_transport(Plan::StartupTransport)]
#[case::startup_invalid_url(Plan::StartupInvalidUrl)]
#[case::startup_integrity_fallback(Plan::StartupIntegrityFallback)]
#[case::startup_parse_fallback(Plan::StartupParseFallback)]
#[case::startup_retry(Plan::StartupRetry)]
#[case::reload_local(Plan::ReloadLocal)]
#[case::reload_remote(Plan::ReloadRemote)]
#[case::reload_miss(Plan::ReloadMiss)]
#[case::reload_shrunk(Plan::ReloadShrunk)]
#[case::reload_unknown_field(Plan::ReloadUnknownField)]
fn catalog_refresh_tests_behavior(#[case] plan: Plan) {
    runtime().block_on(plans::run_plan(plan));
}

fn read_until_stopped(
    service: &TestService,
    stop: &StopFlag,
    reads: &AtomicUsize,
    revisions: (&str, &str),
) {
    while !stop.stopped() {
        let snapshot = service.current.load();
        let prefix = catalog_prefix(
            snapshot.provenance.source_revision.as_deref(),
            revisions.0,
            revisions.1,
        );
        for name in snapshot.catalog.entries.keys() {
            assert!(name.starts_with(prefix), "catalog {prefix} held {name}");
        }
        reads.fetch_add(1, Ordering::SeqCst);
    }
}

fn perform_reloads(runtime: &Runtime, service: &TestService, client: &ScriptedClient) {
    let sleeper = RecordingSleeper::default();
    runtime.block_on(async {
        for _ in 0_u8..5 {
            let outcome = reload_once(service, client, &sleeper).await;
            assert!(matches!(outcome, ReloadOutcome::Reloaded(_)));
        }
    });
}

fn assert_final_catalog(service: &TestService, expected: &str, revisions: (&str, &str)) {
    let snapshot = service.current.load();
    let prefix = catalog_prefix(
        snapshot.provenance.source_revision.as_deref(),
        revisions.0,
        revisions.1,
    );
    assert_eq!(prefix, expected);
    assert!(
        snapshot
            .catalog
            .entries
            .keys()
            .all(|name| name.starts_with(prefix))
    );
}

#[test]
fn concurrent_readers_see_one_complete_catalog() {
    let runtime = runtime();
    let alpha = catalog_body("alpha", 60);
    let beta = catalog_body("beta", 60);
    let service = bundled_service();
    let replies = vec![
        Reply::ok(&alpha),
        Reply::ok(&beta),
        Reply::ok(&alpha),
        Reply::ok(&beta),
        Reply::ok(&alpha),
        Reply::ok(&beta),
    ];
    let client = Arc::new(ScriptedClient::new(replies));
    runtime.block_on(start(&service, Arc::clone(&client)));
    let alpha_revision = blob(&alpha);
    let beta_revision = blob(&beta);
    let reads = AtomicUsize::new(0);
    let stop = StopFlag::new();
    thread::scope(|scope| {
        let reader = scope.spawn(|| {
            read_until_stopped(&service, &stop, &reads, (&alpha_revision, &beta_revision));
        });
        while reads.load(Ordering::SeqCst) == 0 {
            thread::yield_now();
        }
        perform_reloads(&runtime, &service, &client);
        stop.stop();
        reader.join().unwrap();
    });
    assert_eq!(client.requests(), 6);
    assert_final_catalog(&service, "beta", (&alpha_revision, &beta_revision));
}

fn assert_live(registrations: &RecordingRegistrations, expected: &[&str], absent: &[&str]) {
    let live = registrations.live();
    for name in expected {
        assert!(live.contains(*name), "missing {name}");
    }
    for name in absent {
        assert!(!live.contains(*name), "unexpected {name}");
    }
}

#[test]
fn runtime_registrations_replay_after_adoption() {
    let alpha = catalog_body("alpha", 60);
    let beta = catalog_body("beta", 60);
    let registrations = Arc::new(RecordingRegistrations::default());
    let shared = Arc::clone(&registrations);
    let service = registrations_service(shared);
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&alpha),
        Reply::ok(&beta),
        Reply::status(404),
    ]));
    let runtime = runtime();
    runtime.block_on(start(&service, Arc::clone(&client)));
    assert_live(&registrations, &["alpha-00", "runtime-override"], &[]);
    let applied = registrations.applied.load(Ordering::SeqCst);
    let sleeper = RecordingSleeper::default();
    runtime.block_on(reload_once(&service, &client, &sleeper));
    assert_live(
        &registrations,
        &["beta-00", "runtime-override"],
        &["alpha-00"],
    );
    assert_eq!(
        registrations.applied.load(Ordering::SeqCst),
        applied.saturating_add(1)
    );
    runtime.block_on(reload_once(&service, &client, &sleeper));
    assert_eq!(
        registrations.applied.load(Ordering::SeqCst),
        applied.saturating_add(1)
    );
    assert_live(
        &registrations,
        &["beta-00", "runtime-override"],
        &["alpha-00"],
    );
}

#[test]
fn local_mode_never_calls_the_client() {
    let service = local_service();
    let client = Arc::new(ScriptedClient::new(Vec::new()));
    runtime().block_on(async {
        let startup = start(&service, Arc::clone(&client)).await;
        assert!(startup.retry.is_none());
        let sleeper = RecordingSleeper::default();
        let outcome = reload_once(&service, &client, &sleeper).await;
        assert!(matches!(outcome, ReloadOutcome::Reloaded(_)));
        assert_eq!(client.requests(), 0);
    });
}

#[test]
fn fetch_limits_default_to_five_seconds_and_three_attempts() {
    assert_eq!(
        FetchLimits::default(),
        FetchLimits {
            timeout: Duration::from_secs(5),
            max_attempts: 3,
        }
    );
}

#[test]
fn a_single_attempt_budget_starts_no_retry() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::transport(
        "connection refused",
    )]));
    runtime().block_on(async {
        let startup = service
            .startup(
                limits(1),
                Arc::clone(&client),
                Arc::new(RecordingSleeper::default()),
                ScriptedJitter::new(&[]),
            )
            .await;
        assert!(startup.retry.is_none());
        assert_eq!(startup.snapshot.catalog.entries.len(), 60);
        assert!(Arc::ptr_eq(&service.current.load(), &startup.snapshot));
        assert_eq!(client.requests(), 1);
    });
}

#[test]
fn retry_task_honors_attempt_and_wait_limits() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::status(503),
        Reply::status(503),
        Reply::status(503),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    runtime().block_on(async {
        let startup = service
            .startup(
                limits(3),
                Arc::clone(&client),
                Arc::clone(&sleeper),
                ScriptedJitter::new(&[decimal("0.5"), decimal("0.25")]),
            )
            .await;
        let outcome = startup.retry.unwrap().run(Arc::clone(&service)).await;
        assert!(matches!(outcome, RetryOutcome::Kept));
        assert_eq!(client.requests(), 3);
        assert_eq!(sleeper.recorded(), [decimal("2.5"), decimal("4.25")]);
        assert_eq!(
            client.timeouts(),
            [
                Duration::from_secs(5),
                Duration::from_secs(5),
                Duration::from_secs(5)
            ]
        );
        assert!(service.current.load().provenance.fallback_reason.is_some());
    });
}

#[test]
fn an_oversized_catalog_is_not_adopted() {
    let first = catalog_body("first", 60);
    let filler = "x".repeat(0x0100_0001_usize);
    let entry = format!("\"huge-note\":{{\"litellm_provider\":\"openai\",\"note\":\"{filler}\"}}");
    let oversized = body_with_entry("huge", &entry);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&first),
        Reply::ok(&oversized),
    ]));
    runtime().block_on(async {
        let startup = start(&service, Arc::clone(&client)).await;
        let before = startup.snapshot;
        let sleeper = RecordingSleeper::default();
        let outcome = reload_once(&service, &client, &sleeper).await;
        match outcome {
            ReloadOutcome::Unavailable(unavailable) => {
                assert!(unavailable.reason.contains("failed to load"));
            }
            ReloadOutcome::Reloaded(_) => panic!("an oversized catalog must not adopt"),
        }
        assert!(Arc::ptr_eq(&before, &service.current.load()));
    });
}

#[tokio::test]
async fn dropping_the_refresh_task_keeps_the_loaded_catalog() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::status(503)]));
    let sleeper = Arc::new(BlockingSleeper::default());
    let startup = service
        .startup(
            limits(3),
            Arc::clone(&client),
            Arc::clone(&sleeper),
            ScriptedJitter::new(&[]),
        )
        .await;
    let before = startup.snapshot;
    let mut tasks = JoinSet::new();
    let _abort = tasks.spawn(startup.retry.unwrap().run(Arc::clone(&service)));
    sleeper.entered.notified().await;
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    assert!(Arc::ptr_eq(&before, &service.current.load()));
    assert_eq!(client.requests(), 1);
}

#[test]
fn a_shrunk_fetched_map_is_validated_against_the_bundled_count() {
    let fetched = catalog_body("fetched", 60);
    let service = wide_bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::ok(&fetched)]));
    runtime().block_on(async {
        let outcome = start(&service, Arc::clone(&client)).await;
        assert_eq!(
            outcome.snapshot.provenance.fallback_reason.as_deref(),
            Some("Remote data failed integrity validation")
        );
    });
}

#[test]
fn fetched_unknown_fields_are_logged_with_their_paths() {
    let first = catalog_body("first", 60);
    let unknown = body_with_entry(
        "unknown",
        "\"unknown-bad\":{\"litellm_provider\":\"openai\",\"future_flag\":true}",
    );
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&first),
        Reply::ok(&unknown),
    ]));
    let runtime = runtime();
    runtime.block_on(start(&service, Arc::clone(&client)));
    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = fmt()
        .with_ansi(false)
        .with_max_level(Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let sleeper = RecordingSleeper::default();
    let outcome = with_default(subscriber, || {
        runtime.block_on(reload_once(&service, &client, &sleeper))
    });
    let snapshot = adopted(outcome);
    let text = log.text();
    assert!(text.contains(URL), "{text}");
    assert!(text.contains("unknown-bad.future_flag"), "{text}");
    assert!(
        snapshot
            .catalog
            .warnings
            .contains(&"unknown-bad.future_flag".to_owned())
    );
}
