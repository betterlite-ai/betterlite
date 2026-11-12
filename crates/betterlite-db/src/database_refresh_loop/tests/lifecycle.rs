use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::yield_now;
use tokio::time::advance;
use tracing::subscriber::set_default;

use super::fixtures::fakes::{EXPIRY_100, EXPIRY_900, UNPARSABLE_TOKEN_URL, lock, token_url};
use super::fixtures::logs::capture_logs;
use super::fixtures::loops::{FakeLoop, LoopSpec, MintOutcome, build_loop, install_finished_task};
use crate::database_tokens::expiry::TokenAuthKind;

fn installed_stop(built: &FakeLoop) -> Arc<Notify> {
    let task = lock(&built.task);
    let stop = task.as_ref().map(|installed| Arc::clone(&installed.stop));
    drop(task);
    stop.expect("a refresh task is installed")
}

#[tokio::test(start_paused = true)]
async fn disabled_start() {
    let setup = build_loop(LoopSpec {
        token_auth: None,
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    setup.built.start();
    yield_now().await;
    drop(guard);

    assert!(!setup.built.has_task());
    assert!(logs.has(
        "DEBUG",
        "Database token auth not enabled, skipping token refresh task"
    ));
    assert_eq!(logs.count("refresh loop started"), 0);
}

#[tokio::test(start_paused = true)]
async fn duplicate_handle_start() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.built.start();
    yield_now().await;
    assert!(setup.built.has_task());
    let running = installed_stop(&setup.built);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    setup.built.start();
    yield_now().await;
    drop(guard);
    assert!(logs.has("DEBUG", "Token refresh task already running"));
    assert!(Arc::ptr_eq(&installed_stop(&setup.built), &running));
    setup.built.stop().await.expect("the running loop stops");

    let finished = install_finished_task(&setup.built).await;
    let (done_logs, done_subscriber) = capture_logs();
    let done_guard = set_default(done_subscriber);
    setup.built.start();
    yield_now().await;
    drop(done_guard);
    assert!(done_logs.has("DEBUG", "Token refresh task already running"));
    assert!(Arc::ptr_eq(&installed_stop(&setup.built), &finished));
}

#[tokio::test(start_paused = true)]
async fn loop_log_levels() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    setup.built.start();
    yield_now().await;
    assert!(logs.has(
        "INFO",
        "[writer] Started RDS IAM token proactive refresh background task"
    ));
    assert!(logs.has(
        "INFO",
        "[writer] RDS IAM token refresh loop started. Tokens will be refreshed 180s before expiration."
    ));
    assert!(logs.has(
        "INFO",
        "[writer] RDS IAM token refresh scheduled in 30 seconds (0.5 minutes)"
    ));

    advance(Duration::from_secs(30)).await;
    yield_now().await;
    assert!(logs.has("INFO", "[writer] Proactively refreshing RDS IAM token..."));
    assert!(logs.has("INFO", "[writer] RDS IAM token refreshed successfully."));
    assert!(logs.has(
        "INFO",
        "[writer] RDS IAM token refresh scheduled in 720 seconds (12.0 minutes)"
    ));
    assert_eq!(logs.count("WARN"), 0);

    setup.built.stop().await.expect("the loop stops");
    assert!(logs.has(
        "INFO",
        "[writer] Stopped RDS IAM token refresh background task"
    ));
    drop(guard);
}

#[tokio::test(start_paused = true)]
async fn fixed_schedule_no_jitter() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    setup.built.start();
    yield_now().await;
    advance(Duration::from_mins(12)).await;
    yield_now().await;
    advance(Duration::from_mins(12)).await;
    yield_now().await;
    drop(guard);

    assert_eq!(
        logs.count("refresh scheduled in 720 seconds (12.0 minutes)"),
        3
    );
    assert_eq!(logs.count("refresh scheduled in"), 3);
    assert_eq!(logs.count("Proactively refreshing"), 2);
    setup.built.stop().await.expect("the loop stops");
}

#[tokio::test(start_paused = true)]
async fn error_sleep_600() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(UNPARSABLE_TOKEN_URL.to_owned()),
        mint: MintOutcome::Fail,
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    setup.built.start();
    yield_now().await;
    assert!(logs.has(
        "DEBUG",
        "Could not parse token expiration, using fallback interval of 600s"
    ));
    assert!(logs.has(
        "INFO",
        "[writer] RDS IAM token refresh scheduled in 600 seconds (10.0 minutes)"
    ));

    advance(Duration::from_mins(10)).await;
    yield_now().await;
    assert_eq!(logs.count("Proactively refreshing"), 1);
    assert!(logs.has(
        "ERROR",
        "[writer] Error in RDS IAM token refresh loop: the database token could not be minted. Retrying in 600s..."
    ));

    advance(Duration::from_secs(599)).await;
    yield_now().await;
    assert_eq!(logs.count("Proactively refreshing"), 1);
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(logs.count("refresh scheduled in 600 seconds"), 2);
    advance(Duration::from_mins(10)).await;
    yield_now().await;
    assert_eq!(logs.count("Proactively refreshing"), 2);
    drop(guard);

    setup.built.stop().await.expect("the loop stops");
}

#[tokio::test(start_paused = true)]
async fn cancel_normal_vs_error_sleep() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    setup.built.start();
    yield_now().await;
    setup.built.stop().await.expect("the loop stops");
    drop(guard);
    assert!(logs.has("INFO", "RDS IAM token refresh loop cancelled"));
    assert!(!setup.built.has_task());

    let error_setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(UNPARSABLE_TOKEN_URL.to_owned()),
        mint: MintOutcome::Fail,
        on_replaced: None,
    })
    .await;
    let (error_logs, error_subscriber) = capture_logs();
    let error_guard = set_default(error_subscriber);
    error_setup.built.start();
    yield_now().await;
    advance(Duration::from_mins(10)).await;
    yield_now().await;
    assert!(error_logs.has("ERROR", "Error in RDS IAM token refresh loop"));
    error_setup
        .built
        .stop()
        .await
        .expect("the loop stops during the error sleep");
    drop(error_guard);
    assert_eq!(error_logs.count("refresh loop cancelled"), 0);
}

#[tokio::test(start_paused = true)]
async fn stop_clears_handle() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.built.start();
    yield_now().await;
    assert!(setup.built.has_task());
    setup.built.stop().await.expect("the first stop");
    assert!(!setup.built.has_task());
    setup
        .built
        .stop()
        .await
        .expect("a stop without a task is a no-op");
    setup.built.start();
    yield_now().await;
    assert!(setup.built.has_task());
    setup.built.stop().await.expect("the second stop");
    assert!(!setup.built.has_task());
}

#[tokio::test(start_paused = true)]
async fn shutdown_joins() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(TokenAuthKind::Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    setup.built.start();
    yield_now().await;
    setup.built.stop().await.expect("the loop joins");
    let after_stop = logs.contents().len();
    advance(Duration::from_secs(10_000)).await;
    yield_now().await;
    drop(guard);
    assert_eq!(logs.contents().len(), after_stop);
    assert!(!setup.built.has_task());
}
