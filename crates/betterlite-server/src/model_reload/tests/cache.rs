use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::Level;

use crate::model_reload::cache::{
    ReconcileOutcome, ReloadFailure, clear_cache as clear_router_cache,
};

use super::support::{
    Database, FakeRouter, capture_logs, deployment, ids, repository_without_client,
};

fn outcome(desired: &[&str], live: &[&str]) -> ReconcileOutcome {
    ReconcileOutcome {
        still_desired: Some(ids(desired)),
        live_after: Some(ids(live)),
    }
}

#[tokio::test]
async fn clear_cache() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let models = [
        deployment("config", "cfg-1", "openai/gpt-4o", false),
        deployment("plain-db", "db-1", "openai/gpt-4o", true),
        deployment("auto-name", "db-2", "auto_router/fast", true),
        deployment("other-auto", "db-3", "auto_router/slow", true),
    ];
    let mut router = FakeRouter::new(&models, &["auto-name", "other-auto", "kept"]);
    router.reload_result = Ok(outcome(
        &["cfg-1", "db-1", "db-2", "db-3"],
        &["cfg-1", "db-1"],
    ));
    let gate = Arc::new(Mutex::new(()));
    router.gate = Some(Arc::clone(&gate));
    let database = Database::new();
    let repository = database.repository();
    let returned = clear_router_cache(&gate, Some(&mut router), Some(&repository)).await;
    assert_eq!(
        returned,
        outcome(&["cfg-1", "db-1", "db-2", "db-3"], &["cfg-1", "db-1"])
    );
    assert_eq!(router.deleted, vec!["db-2".to_owned(), "db-3".to_owned()]);
    assert_eq!(
        router.remaining(),
        vec!["cfg-1".to_owned(), "db-1".to_owned()]
    );
    assert_eq!(router.strategies.auto, ids(&["kept"]));
    assert_eq!(router.strategies.complexity, ids(&["kept"]));
    assert_eq!(router.strategies.adaptive, ids(&["kept"]));
    assert_eq!(router.strategies.quality, ids(&["kept"]));
    assert_eq!(router.reloads, 1);
    assert!(router.database_connected);
    assert!(router.gate_held);
    let text = logs.contents();
    assert!(text.contains("Clearing only DB models, preserving config models"));
    assert!(text.contains("Reconciled 3 DB models, preserved 1 config models"));
}

#[tokio::test]
async fn clear_cache_reload_failure() {
    let (logs, _guard) = capture_logs(Level::ERROR);
    let models = [deployment("auto-name", "db-2", "auto_router/fast", true)];
    let mut router = FakeRouter::new(&models, &["auto-name"]);
    router.reload_result = Err(ReloadFailure {
        message: "database is down".to_owned(),
    });
    let gate = Mutex::new(());
    let database = Database::new();
    let repository = database.repository();
    let returned = clear_router_cache(&gate, Some(&mut router), Some(&repository)).await;
    assert_eq!(returned, ReconcileOutcome::default());
    assert_eq!(router.deleted, vec!["db-2".to_owned()]);
    assert_eq!(router.reloads, 1);
    assert!(
        logs.contents()
            .contains("Failed to clear cache and reload models. Due to error - database is down")
    );
}

#[tokio::test]
async fn clear_cache_skips_without_router() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let gate = Mutex::new(());
    let database = Database::new();
    let repository = database.repository();
    let returned = clear_router_cache::<FakeRouter>(&gate, None, Some(&repository)).await;
    assert_eq!(returned, ReconcileOutcome::default());
    assert!(
        logs.contents()
            .contains("llm_router or prisma_client is None, skipping cache clear")
    );
}

#[tokio::test]
async fn clear_cache_skips_without_database() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let mut router = FakeRouter::new(&[], &[]);
    let gate = Mutex::new(());
    let returned = clear_router_cache(&gate, Some(&mut router), None).await;
    assert_eq!(returned, ReconcileOutcome::default());
    assert_eq!(router.reloads, 0);
    assert!(router.deleted.is_empty());
    assert!(
        logs.contents()
            .contains("llm_router or prisma_client is None, skipping cache clear")
    );
}

#[tokio::test]
async fn clear_cache_skips_without_a_database_client() {
    let (logs, _guard) = capture_logs(Level::DEBUG);
    let mut router = FakeRouter::new(&[], &[]);
    let gate = Mutex::new(());
    let repository = repository_without_client();
    let returned = clear_router_cache(&gate, Some(&mut router), Some(&repository)).await;
    assert_eq!(returned, ReconcileOutcome::default());
    assert_eq!(router.reloads, 0);
    assert!(
        logs.contents()
            .contains("llm_router or prisma_client is None, skipping cache clear")
    );
}
