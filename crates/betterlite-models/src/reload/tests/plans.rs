use std::sync::Arc;
use std::time::Duration;

use crate::reload::{ReloadOutcome, RetryOutcome, StartupOutcome};
use crate::source::{CatalogProvenance, SourceKind};

use super::support::{
    RecordingSleeper, Reply, ScriptedClient, ScriptedJitter, TestService, URL, blob,
    body_with_entry, bundled_service, catalog_body, fixed_moment, limits, local_service,
};
use super::{adopted, decimal, reload_once, start};

#[derive(Clone, Copy, Debug)]
pub(super) enum Plan {
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

pub(super) async fn run_plan(plan: Plan) {
    match plan {
        Plan::StartupLocal => plan_startup_local().await,
        Plan::StartupRemote => plan_startup_remote().await,
        Plan::StartupFallback => plan_startup_fallback().await,
        Plan::StartupTransport => {
            plan_startup_failure(Reply::transport("connection refused"), Fallback::Transport).await;
        }
        Plan::StartupInvalidUrl => plan_startup_invalid_url().await,
        Plan::StartupIntegrityFallback => {
            let shrunk = catalog_body("shrunk", 20);
            plan_startup_failure(Reply::ok(&shrunk), Fallback::Integrity).await;
        }
        Plan::StartupParseFallback => {
            let broken = body_with_entry(
                "broken",
                "\"broken-bad\":{\"litellm_provider\":\"openai\",\"mode\":5}",
            );
            plan_startup_failure(Reply::ok(&broken), Fallback::Parse).await;
        }
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
    assert_published(&service, &outcome);
    assert!(outcome.retry.is_none());
    assert_eq!(client.requests(), 0);
}

type Startup = StartupOutcome<ScriptedClient, RecordingSleeper, ScriptedJitter>;

fn assert_published(service: &TestService, outcome: &Startup) {
    assert_eq!(outcome.snapshot.catalog.entries.len(), 60);
    assert!(Arc::ptr_eq(&service.current.load(), &outcome.snapshot));
}

async fn started(replies: Vec<Reply>) -> (Arc<TestService>, Startup, Arc<ScriptedClient>) {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(replies));
    let outcome = start(&service, Arc::clone(&client)).await;
    (service, outcome, client)
}

fn assert_remote(outcome: &Startup, body: &str) {
    let expected = CatalogProvenance {
        kind: SourceKind::Remote,
        url: Some(URL.to_owned()),
        is_env_forced: false,
        fallback_reason: None,
        loaded_at: Some(fixed_moment()),
        source_revision: Some(blob(body)),
        etag: Some("\"v1\"".to_owned()),
    };
    assert_eq!(outcome.snapshot.provenance, expected);
    assert_eq!(outcome.snapshot.catalog.entries.len(), 60);
}

fn assert_fallback(service: &TestService, outcome: &Startup) {
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
    assert_published(service, outcome);
    assert!(outcome.retry.is_none());
}

async fn plan_startup_remote() {
    let body = catalog_body("remote", 60);
    let (service, outcome, client) = started(vec![Reply::ok_with_etag(&body, "\"v1\"")]).await;
    assert_remote(&outcome, &body);
    assert_published(&service, &outcome);
    assert_eq!(client.requests(), 1);
    assert_eq!(client.timeouts(), [Duration::from_secs(5)]);
}

async fn plan_startup_fallback() {
    let (service, outcome, client) = started(vec![Reply::status(404)]).await;
    assert_fallback(&service, &outcome);
    assert_eq!(client.requests(), 1);
}

enum Fallback {
    Transport,
    Integrity,
    Parse,
}

async fn plan_startup_failure(reply: Reply, fallback: Fallback) {
    let service = bundled_service();
    let client = Arc::new(ScriptedClient::new(vec![reply]));
    let outcome = start(&service, Arc::clone(&client)).await;
    let provenance = &outcome.snapshot.provenance;
    match fallback {
        Fallback::Transport => {
            assert!(outcome.retry.is_some());
            let reason = provenance.fallback_reason.as_deref().unwrap();
            assert!(reason.starts_with("Remote fetch failed:"));
            assert!(reason.contains(URL));
        }
        Fallback::Integrity => {
            assert!(outcome.retry.is_none());
            assert_eq!(
                provenance.fallback_reason.as_deref(),
                Some("Remote data failed integrity validation")
            );
        }
        Fallback::Parse => {
            assert!(outcome.retry.is_none());
            assert_eq!(provenance.kind, SourceKind::Local);
            assert!(
                provenance
                    .fallback_reason
                    .as_deref()
                    .is_some_and(|reason| reason.starts_with("Remote data failed to load:"))
            );
        }
    }
    assert_published(&service, &outcome);
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
    assert_published(&service, &outcome);
    assert_eq!(client.requests(), 1);
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
    let retry_outcome = outcome.retry.unwrap().run(Arc::clone(&service)).await;
    assert!(matches!(retry_outcome, RetryOutcome::Adopted(_)));
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
