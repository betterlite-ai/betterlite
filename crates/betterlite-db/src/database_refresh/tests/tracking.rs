use std::future::ready;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::DateTime;
use rstest::rstest;
use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::database_refresh::access::{self, AccessError};
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::generation::{GenerationError, GenerationOptions};
use crate::database_refresh::handle::PoolProvider as _;
use crate::database_refresh::handle::TransactionEnd;
use crate::database_refresh::retirement;
use crate::database_refresh::tracker::{DrainTracker, TransactionId};
use crate::database_tokens::expiry::TokenAuthKind;

use super::support::{
    ConnectStep, FakeError, FakeGeneration, FakeProvider, FakeScript, QueryStep, TestTransaction,
    create, lock, options, refresh, secret, stop, token_options, transaction, wait_for_event,
};

#[test]
fn constructor_defaults() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let built = FakeGeneration::new(provider, GenerationOptions::default(), None, None, None)
        .expect("the default generation builds");
    assert_eq!(built.observe().generation, 0);
    assert!(built.observe().handle.is_none());
    assert!(!built.observe().connected);
    assert!(lock(&built.last_refresh_time).is_none());
    assert!(built.options.token_auth.is_none());
    assert!(!built.iam_token_db_auth());
    assert_eq!(built.token_label(), "database token");
    assert_eq!(built.options.db_url_env_var, "DATABASE_URL");
    assert_eq!(built.options.log_prefix, "");
    assert!(built.options.iam_endpoint.is_none());
    assert!(!built.options.recreate_uses_datasource);
    assert_eq!(retirement::retirement_count(&built), 0);
    assert!(matches!(
        access::operation(&built),
        Err(AccessError::NotConnected)
    ));
}

#[test]
fn refresh_time_recorded() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let built = FakeGeneration::new(provider, GenerationOptions::default(), None, None, None)
        .expect("the default generation builds");
    assert!(lock(&built.last_refresh_time).is_none());
    let at = DateTime::from_timestamp(1_700_000_000, 0).expect("a timestamp");
    built.record_refresh(at);
    assert_eq!(*lock(&built.last_refresh_time), Some(at));
}

#[test]
fn explicit_strategy_over_legacy_flag() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let refresh = Arc::new(refresh());
    let mut settings = options(DatabaseUrlSlot::default());
    settings.iam_token_db_auth = true;
    settings.token_auth = Some(TokenAuthKind::Azure);
    let built = FakeGeneration::new(provider, settings, None, Some(refresh), None)
        .expect("the explicit strategy builds");
    assert_eq!(built.options.token_auth, Some(TokenAuthKind::Azure));
    assert_eq!(built.token_label(), "Azure Entra token");
}

#[test]
fn azure_counts_as_token_auth() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let refresh = Arc::new(refresh());
    let mut settings = options(DatabaseUrlSlot::default());
    settings.token_auth = Some(TokenAuthKind::Azure);
    let built = FakeGeneration::new(provider, settings, None, Some(refresh), None)
        .expect("the Azure strategy builds");
    assert!(built.iam_token_db_auth());
    assert_eq!(built.token_label(), "Azure Entra token");
}

#[rstest]
#[case::plain("writer", "writer ")]
#[case::trailing_space("writer ", "writer  ")]
#[case::empty("", "")]
fn log_prefix_spacing(#[case] prefix: &str, #[case] expected: &str) {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let mut settings = options(DatabaseUrlSlot::default());
    settings.log_prefix = prefix.to_owned();
    let built = FakeGeneration::new(provider, settings, None, None, None)
        .expect("the prefixed generation builds");
    assert_eq!(built.options.log_prefix, expected);
}

#[tokio::test]
async fn query_guard_on_error_and_cancel() {
    let script = Arc::new(FakeScript::default());
    script.script_query("fails", QueryStep::Fail);
    let gate = Arc::new(Notify::new());
    script.script_query("blocks", QueryStep::Blocked);
    script.gate_query("blocks", Arc::clone(&gate));
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    handle.query("fails").await.unwrap_err();
    assert_eq!(tracker.status().active_operations, 0);
    assert!(tracker.status().drained);
    assert_eq!(handle.query("succeeds").await.expect("a query"), 0);
    assert!(tracker.status().drained);
    let mut tasks = JoinSet::new();
    let querying = access::operation(&built).expect("a connected operation");
    tasks.spawn(async move { querying.query("blocks").await });
    wait_for_event(&script, "query:blocks").await;
    assert_eq!(tracker.status().active_operations, 1);
    stop(&mut tasks).await;
    assert_eq!(tracker.status().active_operations, 0);
    assert!(tracker.status().drained);
}

#[tokio::test]
async fn transaction_pins_idle_generation() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    script.script_query("inside", QueryStep::Blocked);
    script.gate_query("inside", Arc::clone(&gate));
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let opened = transaction(&script);
    let held = handle
        .begin_transaction(ready(Ok::<TestTransaction, FakeError>(opened)))
        .await
        .expect("a transaction begins");
    assert_eq!(tracker.status().active_operations, 1);
    assert!(!tracker.status().drained);
    let mut tasks = JoinSet::new();
    let querying = access::operation(&built).expect("a connected operation");
    tasks.spawn(async move { querying.query("inside").await });
    wait_for_event(&script, "query:inside").await;
    assert_eq!(tracker.status().active_operations, 2);
    stop(&mut tasks).await;
    assert_eq!(tracker.status().active_operations, 1);
    handle
        .finish_transaction(held, TransactionEnd::Commit)
        .await
        .expect("the transaction commits");
    assert!(tracker.status().drained);
    assert!(script.has_event("commit"));
}

#[tokio::test]
async fn begin_failure_releases_guard() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let outcome = handle
        .begin_transaction(ready(Err::<TestTransaction, FakeError>(FakeError::Query)))
        .await;
    assert!(outcome.is_err());
    assert_eq!(tracker.status().active_operations, 0);
    assert!(tracker.status().drained);
}

#[tokio::test]
async fn commit_failure_releases_transaction() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let mut opened = transaction(&script);
    opened.outcome = Some(Err(FakeError::Query));
    let held = handle
        .begin_transaction(ready(Ok::<TestTransaction, FakeError>(opened)))
        .await
        .expect("a transaction begins");
    let outcome = handle
        .finish_transaction(held, TransactionEnd::Commit)
        .await;
    assert!(outcome.is_err());
    assert_eq!(tracker.status().active_operations, 0);
    assert_eq!(tracker.status().transactions, 0);
    assert!(tracker.status().drained);
}

#[tokio::test]
async fn rollback_cancel_releases_transaction() {
    let script = Arc::new(FakeScript::default());
    let gate = Arc::new(Notify::new());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let mut opened = transaction(&script);
    opened.gate = Some(Arc::clone(&gate));
    let held = handle
        .begin_transaction(ready(Ok::<TestTransaction, FakeError>(opened)))
        .await
        .expect("a transaction begins");
    let mut tasks = JoinSet::new();
    let rolling = access::operation(&built).expect("a connected operation");
    tasks.spawn(async move {
        rolling
            .finish_transaction(held, TransactionEnd::Rollback)
            .await
    });
    wait_for_event(&script, "rollback").await;
    assert_eq!(tracker.status().active_operations, 2);
    stop(&mut tasks).await;
    assert_eq!(tracker.status().active_operations, 0);
    assert_eq!(tracker.status().transactions, 0);
    assert!(tracker.status().drained);
}

#[test]
fn unknown_transaction_no_underflow() {
    let tracker = Arc::new(DrainTracker::new());
    assert!(!tracker.finish_transaction(TransactionId(7)));
    assert_eq!(tracker.status().active_operations, 0);
    assert!(tracker.status().drained);
    let held = tracker.begin_transaction();
    assert!(!tracker.finish_transaction(TransactionId(99)));
    assert_eq!(tracker.status().active_operations, 1);
    assert!(tracker.finish_transaction(held.id));
    drop(held);
    assert_eq!(tracker.status().active_operations, 0);
    assert_eq!(tracker.status().transactions, 0);
    assert!(tracker.status().drained);
}

#[tokio::test]
async fn tracker_reused() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        true,
    )
    .await;
    let current = built.observe().handle.expect("the initial handle");
    let first = built.instrument(&current).expect("a tracker");
    built.connect(None).await.expect("a second connect");
    let second = built.instrument(&current).expect("the reused tracker");
    assert!(Arc::ptr_eq(&first, &second));
    let stray_provider = FakeProvider {
        script: Arc::clone(&script),
    };
    let stray = Arc::new(
        stray_provider
            .build(&secret("postgres://stray"), None)
            .expect("a stray handle"),
    );
    assert!(built.instrument(&stray).is_none());
}

#[tokio::test]
async fn connect_timeout_forwarding() {
    let script = Arc::new(FakeScript::default());
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        false,
    )
    .await;
    built
        .connect(Some(Duration::from_secs(42)))
        .await
        .expect("a connect with a timeout");
    assert!(built.observe().connected);
    built.connect(None).await.expect("a connect without one");
    assert_eq!(
        script.connect_timeouts(),
        [Some(Duration::from_secs(42)), None]
    );
}

#[tokio::test]
async fn connect_failure_installs_no_tracker() {
    let script = Arc::new(FakeScript::default());
    script.script_connect(0, ConnectStep::Fail);
    let built = create(
        &script,
        options(DatabaseUrlSlot::default()),
        None,
        None,
        false,
    )
    .await;
    assert!(built.connect(None).await.is_err());
    let handle = access::operation(&built).expect("the handle stays selected");
    assert!(handle.tracker.is_none());
    assert!(!built.observe().connected);
}

#[test]
fn refresh_port_required() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let outcome = FakeGeneration::new(
        provider,
        token_options(DatabaseUrlSlot::default()),
        None,
        None,
        None,
    );
    assert!(matches!(
        outcome.err(),
        Some(GenerationError::RefreshPortRequired)
    ));
}

#[test]
fn missing_attribute_before_refresh() {
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(FakeProvider { script });
    let fake = refresh();
    fake.expired.store(true, Ordering::SeqCst);
    let refresh = Arc::new(fake);
    let built = FakeGeneration::new(
        provider,
        token_options(DatabaseUrlSlot::default()),
        None,
        Some(Arc::clone(&refresh)),
        None,
    )
    .expect("the token generation builds");
    assert!(matches!(
        access::operation(&built),
        Err(AccessError::NotConnected)
    ));
    assert_eq!(refresh.expired_checks.load(Ordering::SeqCst), 0);
    assert_eq!(refresh.refreshes.load(Ordering::SeqCst), 0);
}
