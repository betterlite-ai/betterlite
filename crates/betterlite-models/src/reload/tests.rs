#![cfg(test)]

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rstest::rstest;
use rust_decimal::Decimal;
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinSet;

use crate::reload::{ReloadOutcome, RetryOutcome, StartupOutcome};
use crate::source::{CatalogSnapshot, SourceKind};

use support::{
    BlockingSleeper, RecordingRegistrations, RecordingSleeper, Reply, ScriptedClient,
    ScriptedJitter, StopFlag, TestService, URL, blob, body_with_entry, bundled_service,
    catalog_body, fixed_moment, limits, local_service, registrations_service, wide_bundled_service,
};

#[derive(Clone, Copy, Debug)]
enum Plan {
    StartupLocal,
    StartupRemote,
    StartupFallback,
    StartupTransport,
    StartupInvalidUrl,
    StartupIntegrityFallback,
    StartupParseFallback,
    StartupRetry,
    ReloadLocal,
    ReloadRemote,
    ReloadMiss,
    ReloadShrunk,
    ReloadUnknownField,
}

fn runtime() -> Runtime {
    Builder::new_current_thread().build().unwrap()
}

fn decimal(text: &str) -> Decimal {
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

async fn start(
    service: &TestService,
    client: Arc<ScriptedClient>,
) -> StartupOutcome<ScriptedClient, RecordingSleeper, ScriptedJitter> {
    service
        .startup(
            limits(1),
            client,
            Arc::new(RecordingSleeper::default()),
            ScriptedJitter::new(&[]),
        )
        .await
}

async fn reload_once(
    service: &TestService,
    client: &ScriptedClient,
    sleeper: &RecordingSleeper,
) -> ReloadOutcome {
    service
        .reload(limits(1), client, sleeper, &mut ScriptedJitter::new(&[]))
        .await
}

fn adopted(outcome: ReloadOutcome) -> Arc<CatalogSnapshot> {
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
    runtime().block_on(run_plan(plan));
}

async fn run_plan(plan: Plan) {
    match plan {
        Plan::StartupLocal => plan_startup_local().await,
        Plan::StartupRemote => plan_startup_remote().await,
        Plan::StartupFallback => plan_startup_fallback().await,
        Plan::StartupTransport => plan_startup_transport().await,
        Plan::StartupInvalidUrl => plan_startup_invalid_url().await,
        Plan::StartupIntegrityFallback => plan_startup_integrity_fallback().await,
        Plan::StartupParseFallback => plan_startup_parse_fallback().await,
        Plan::StartupRetry => plan_startup_retry().await,
        Plan::ReloadLocal => plan_reload_local().await,
        Plan::ReloadRemote => plan_reload_remote().await,
        Plan::ReloadMiss => plan_reload_miss().await,
        Plan::ReloadShrunk => plan_reload_shrunk().await,
        Plan::ReloadUnknownField => plan_reload_unknown_field().await,
    }
}

async fn plan_startup_local() {
    let revision = blob(&catalog_body("bundled", 60));
    let service = local_service();
    let client = Arc::new(ScriptedClient::new(Vec::new()));
    let outcome = start(&service, Arc::clone(&client)).await;
    let provenance = &outcome.snapshot.provenance;
    assert_eq!(provenance.kind, SourceKind::Local);
    assert_eq!(provenance.url, None);
    assert!(provenance.is_env_forced);
    assert_eq!(provenance.fallback_reason, None);
    assert_eq!(provenance.loaded_at, Some(fixed_moment()));
    assert_eq!(provenance.source_revision, Some(revision));
    assert_eq!(provenance.etag, None);
    assert_eq!(outcome.snapshot.catalog.entries.len(), 60);
    assert!(outcome.retry.is_none());
    assert_eq!(client.requests(), 0);
}

async fn plan_startup_remote() {
    let body = catalog_body("remote", 60);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::ok_with_etag(
        &body, "\"v1\"",
    )]));
    let outcome = start(&service, Arc::clone(&client)).await;
    let provenance = &outcome.snapshot.provenance;
    assert_eq!(provenance.kind, SourceKind::Remote);
    assert_eq!(provenance.url.as_deref(), Some(URL));
    assert!(!provenance.is_env_forced);
    assert_eq!(provenance.fallback_reason, None);
    assert_eq!(provenance.source_revision, Some(blob(&body)));
    assert_eq!(provenance.etag.as_deref(), Some("\"v1\""));
    assert_eq!(outcome.snapshot.catalog.entries.len(), 60);
    assert_eq!(client.requests(), 1);
    assert_eq!(client.timeouts(), [Duration::from_secs(5)]);
}

async fn plan_startup_fallback() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::status(404)]));
    let outcome = start(&service, Arc::clone(&client)).await;
    let provenance = &outcome.snapshot.provenance;
    assert_eq!(provenance.kind, SourceKind::Local);
    assert_eq!(provenance.url.as_deref(), Some(URL));
    assert!(!provenance.is_env_forced);
    assert_eq!(
        provenance.fallback_reason,
        Some(format!("Remote fetch failed: HTTP 404 from {URL}"))
    );
    assert_eq!(provenance.etag, None);
    assert!(provenance.source_revision.is_some());
    assert!(outcome.retry.is_none());
    assert_eq!(client.requests(), 1);
}

async fn plan_startup_transport() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::transport(
        "connection refused",
    )]));
    let outcome = start(&service, Arc::clone(&client)).await;
    assert!(outcome.retry.is_none());
    let reason = outcome
        .snapshot
        .provenance
        .fallback_reason
        .as_deref()
        .unwrap();
    assert!(reason.starts_with("Remote fetch failed:"));
    assert!(reason.contains(URL));
    assert_eq!(client.requests(), 1);
}

async fn plan_startup_invalid_url() {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::InvalidUrl {
        detail: "unsupported scheme".to_owned(),
    }]));
    let outcome = start(&service, Arc::clone(&client)).await;
    assert!(outcome.retry.is_none());
    assert_eq!(outcome.snapshot.provenance.kind, SourceKind::Local);
    assert_eq!(client.requests(), 1);
}

async fn plan_startup_integrity_fallback() {
    let shrunk = catalog_body("shrunk", 20);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::ok(&shrunk)]));
    let outcome = start(&service, Arc::clone(&client)).await;
    assert_eq!(
        outcome.snapshot.provenance.fallback_reason.as_deref(),
        Some("Remote data failed integrity validation")
    );
    assert!(
        !outcome
            .snapshot
            .catalog
            .entries
            .keys()
            .any(|name| name.starts_with("shrunk"))
    );
}

async fn plan_startup_parse_fallback() {
    let broken = body_with_entry(
        "broken",
        "\"broken-bad\":{\"litellm_provider\":\"openai\",\"mode\":5}",
    );
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![Reply::ok(&broken)]));
    let outcome = start(&service, Arc::clone(&client)).await;
    let reason = outcome
        .snapshot
        .provenance
        .fallback_reason
        .as_deref()
        .unwrap();
    assert!(reason.starts_with("Remote data failed to load:"));
    assert_eq!(outcome.snapshot.provenance.kind, SourceKind::Local);
}

async fn plan_startup_retry() {
    let body = catalog_body("retried", 60);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::status(503),
        Reply::ok(&body),
    ]));
    let sleeper = Arc::new(RecordingSleeper::default());
    let outcome = service
        .startup(
            limits(3),
            Arc::clone(&client),
            Arc::clone(&sleeper),
            ScriptedJitter::new(&[decimal("0.5")]),
        )
        .await;
    assert!(outcome.snapshot.provenance.fallback_reason.is_some());
    let adopted = outcome.retry.unwrap().run(Arc::clone(&service)).await;
    assert!(matches!(adopted, RetryOutcome::Adopted(_)));
    let provenance = &service.current.load().provenance;
    assert_eq!(provenance.kind, SourceKind::Remote);
    assert_eq!(provenance.fallback_reason, None);
    assert_eq!(provenance.source_revision, Some(blob(&body)));
    assert_eq!(client.requests(), 2);
    assert_eq!(sleeper.recorded(), [decimal("2.5")]);
}

async fn plan_reload_local() {
    let service = local_service();
    let client = Arc::new(ScriptedClient::new(Vec::new()));
    let sleeper = RecordingSleeper::default();
    let snapshot = adopted(reload_once(&service, &client, &sleeper).await);
    assert_eq!(snapshot.provenance.kind, SourceKind::Local);
    assert_eq!(snapshot.provenance.loaded_at, Some(fixed_moment()));
    assert_eq!(client.requests(), 0);
}

async fn plan_reload_remote() {
    let first = catalog_body("first", 60);
    let second = catalog_body("second", 60);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&first),
        Reply::ok_with_etag(&second, "\"v2\""),
    ]));
    let startup = start(&service, Arc::clone(&client)).await;
    assert_eq!(
        startup.snapshot.provenance.source_revision,
        Some(blob(&first))
    );
    let sleeper = RecordingSleeper::default();
    let snapshot = adopted(reload_once(&service, &client, &sleeper).await);
    assert_eq!(snapshot.provenance.source_revision, Some(blob(&second)));
    assert_eq!(snapshot.provenance.etag.as_deref(), Some("\"v2\""));
    assert!(
        snapshot
            .catalog
            .entries
            .keys()
            .any(|name| name.starts_with("second"))
    );
    assert_eq!(client.requests(), 2);
}

async fn plan_reload_miss() {
    let first = catalog_body("first", 60);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&first),
        Reply::status(404),
    ]));
    let startup = start(&service, Arc::clone(&client)).await;
    let before = startup.snapshot;
    let sleeper = RecordingSleeper::default();
    let outcome = reload_once(&service, &client, &sleeper).await;
    match outcome {
        ReloadOutcome::Unavailable(unavailable) => {
            assert_eq!(unavailable.reason, format!("HTTP 404 from {URL}"));
        }
        ReloadOutcome::Reloaded(_) => panic!("a 404 must not adopt"),
    }
    let after = service.current.load();
    assert!(Arc::ptr_eq(&before, &after));
    assert_eq!(after.provenance.source_revision, Some(blob(&first)));
}

async fn plan_reload_shrunk() {
    let first = catalog_body("first", 60);
    let shrunk = catalog_body("shrunk", 20);
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&first),
        Reply::ok(&shrunk),
    ]));
    let startup = start(&service, Arc::clone(&client)).await;
    let before = startup.snapshot;
    let sleeper = RecordingSleeper::default();
    let outcome = reload_once(&service, &client, &sleeper).await;
    match outcome {
        ReloadOutcome::Unavailable(unavailable) => {
            assert!(unavailable.reason.contains("failed integrity validation"));
        }
        ReloadOutcome::Reloaded(_) => panic!("a shrunken map must not adopt"),
    }
    assert!(Arc::ptr_eq(&before, &service.current.load()));
}

async fn plan_reload_unknown_field() {
    let unknown = body_with_entry(
        "unknown",
        "\"unknown-bad\":{\"litellm_provider\":\"openai\",\"future_flag\":true}",
    );
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![
        Reply::ok(&catalog_body("first", 60)),
        Reply::ok(&unknown),
    ]));
    start(&service, Arc::clone(&client)).await;
    let sleeper = RecordingSleeper::default();
    let snapshot = adopted(reload_once(&service, &client, &sleeper).await);
    assert!(
        snapshot
            .catalog
            .warnings
            .contains(&"unknown-bad.future_flag".to_owned())
    );
    assert!(snapshot.catalog.lookup.contains_key("unknown-bad"));
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
