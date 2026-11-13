use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use secrecy::ExposeSecret as _;
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tracing::subscriber::set_default;

use super::fixtures::fakes::{
    ConnectStep, EXPIRY_100, EXPIRY_900, FakeError, FakeRdsProvider, UNPARSABLE_TOKEN_URL,
    UnusedAzureFactory, lock, naive, slot_url, token_url, utc,
};
use super::fixtures::logs::capture_logs;
use super::fixtures::loops::{LoopSpec, MintOutcome, build_loop, wait_for_event};
use crate::database_refresh::access::RefreshTrigger;
use crate::database_refresh::handle::PoolHandle as _;
use crate::database_refresh::replacement::ReplacementError;
use crate::database_refresh_loop::clock::{RefreshClock as _, SystemClock};
use crate::database_refresh_loop::lifecycle::LoopTask;
use crate::database_refresh_loop::minting::{DatabaseTokenMinter, TokenMinter as _};
use crate::database_refresh_loop::port::RefreshError;
use crate::database_refresh_loop::schedule::{
    FALLBACK_REFRESH_SECONDS, RefreshQuery, is_token_expired, seconds_until_refresh,
    token_refresh_not_needed,
};
use crate::database_tokens::endpoint::IAMEndpoint;
use crate::database_tokens::expiry::TokenAuthKind::{Azure, Rds};
use crate::database_tokens::strategy::{DatabaseTokenAuth, RdsIamTokenAuth};

#[test]
fn expiry_buffer() {
    let url = token_url(EXPIRY_900);
    let schedule = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    assert_eq!(schedule.wait, Duration::from_mins(12));

    let later = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:11:00"),
    })
    .expect("the token parses");
    assert_eq!(later.wait, Duration::from_mins(1));

    let inside_floor = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:11:50"),
    })
    .expect("the token parses");
    assert_eq!(inside_floor.wait, Duration::from_secs(30));
}

#[test]
fn unknown_expiry_fallback() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    let schedule = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(UNPARSABLE_TOKEN_URL),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("an unreadable token falls back");
    assert_eq!(schedule.wait, Duration::from_secs(FALLBACK_REFRESH_SECONDS));
    assert!(logs.has(
        "DEBUG",
        "Could not parse token expiration, using fallback interval of 600s"
    ));

    let missing = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: None,
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("a missing URL falls back");
    assert_eq!(missing.wait, Duration::from_secs(600));
    let disabled_url = token_url(EXPIRY_900);
    let disabled = seconds_until_refresh(&RefreshQuery {
        kind: None,
        token_url: Some(&disabled_url),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("a disabled strategy falls back");
    assert_eq!(disabled.wait, Duration::from_secs(600));
    assert_eq!(logs.count("using fallback interval of 600s"), 3);
}

#[test]
fn expired_and_unreadable_tokens() {
    let (logs, subscriber) = capture_logs();
    let _guard = set_default(subscriber);
    assert!(
        is_token_expired(&RefreshQuery {
            kind: Some(Rds),
            token_url: Some(UNPARSABLE_TOKEN_URL),
            now: naive("2024-01-01T00:00:00"),
        })
        .expect("an unreadable token is expired")
    );
    assert!(logs.has(
        "DEBUG",
        "Could not parse token expiration, treating as expired"
    ));
    assert!(
        is_token_expired(&RefreshQuery {
            kind: Some(Rds),
            token_url: None,
            now: naive("2024-01-01T00:00:00"),
        })
        .expect("no URL is expired")
    );
    assert_eq!(logs.count("treating as expired"), 1);
    assert!(
        !token_refresh_not_needed(&RefreshQuery {
            kind: Some(Rds),
            token_url: Some(UNPARSABLE_TOKEN_URL),
            now: naive("2024-01-01T00:00:00"),
        })
        .expect("an unreadable token is never fresh")
    );
}

#[test]
fn cached_token_minimum_sleep() {
    let url = token_url(EXPIRY_100);
    let first = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    assert_eq!(first.wait, Duration::from_secs(30));
    let second = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    let third = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&url),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    assert_eq!(second.wait, first.wait);
    assert_eq!(third.wait, first.wait);

    let over_url = token_url(EXPIRY_900);
    let just_over = seconds_until_refresh(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&over_url),
        now: naive("2024-01-01T00:11:29.5"),
    })
    .expect("the token parses");
    assert_eq!(just_over.wait, Duration::new(30, 500_000_000));
}

#[tokio::test]
async fn lock_rechecks_freshness() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.script.script_connect(1, ConnectStep::Blocked);
    let gate = Arc::new(Notify::new());
    setup.script.gate_connect(1, Arc::clone(&gate));
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    let mut tasks = JoinSet::new();
    let first = Arc::clone(&setup.built.port);
    tasks.spawn(async move { RefreshTrigger::refresh(first.as_ref()).await });
    wait_for_event(&setup.script, "connect:1").await;
    let second = Arc::clone(&setup.built.port);
    tasks.spawn(async move { RefreshTrigger::refresh(second.as_ref()).await });
    yield_now().await;
    gate.notify_one();
    let mut outcomes = Vec::new();
    while let Some(outcome) = tasks.join_next().await {
        outcomes.push(outcome.expect("the refresh task joins"));
    }
    drop(guard);

    assert!(outcomes.iter().all(Result::is_ok));
    assert_eq!(setup.script.event_count("connect:1"), 1);
    assert_eq!(setup.mint.endpoint_count(), 1);
    assert_eq!(setup.built.generation.observe().generation, 1);
    assert!(logs.has("DEBUG", "still fresh; skipping redundant refresh."));
}

#[tokio::test]
async fn mint_failure_no_publish() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Fail,
        on_replaced: None,
    })
    .await;
    let before = slot_url(&setup.slot);

    let outcome = RefreshTrigger::refresh(setup.built.port.as_ref()).await;

    assert!(matches!(outcome, Err(RefreshError::Mint { .. })));
    assert_eq!(slot_url(&setup.slot), before);
    assert_eq!(setup.built.generation.observe().generation, 0);
    assert!(setup.initial.is_connected());
    assert_eq!(setup.script.build_urls().len(), 1);
}

#[tokio::test]
async fn falsey_mint_no_timestamp() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Missing,
        on_replaced: None,
    })
    .await;
    let before = slot_url(&setup.slot);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    let outcome = RefreshTrigger::refresh(setup.built.port.as_ref()).await;
    drop(guard);

    if let Err(error) = outcome {
        panic!("the falsey mint returns normally: {error}");
    }
    assert_eq!(*lock(&setup.built.generation.last_refresh_time), None);
    assert_eq!(setup.built.generation.observe().generation, 0);
    assert_eq!(slot_url(&setup.slot), before);
    assert!(logs.has(
        "ERROR",
        "[writer] Failed to generate new RDS IAM token during proactive refresh"
    ));
}

#[tokio::test]
async fn candidate_failure_restores_url() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.script.script_connect(1, ConnectStep::Fail);
    let before = slot_url(&setup.slot);

    let outcome = RefreshTrigger::refresh(setup.built.port.as_ref()).await;

    assert!(matches!(
        outcome,
        Err(RefreshError::Replace {
            source: ReplacementError::Connect { .. }
        })
    ));
    assert_eq!(slot_url(&setup.slot), before);
    assert_eq!(setup.built.generation.observe().generation, 0);
    assert!(setup.script.has_event("close:1"));
    assert!(setup.initial.is_connected());
}

#[tokio::test]
async fn cancel_restores_absent_url() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: None,
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.script.script_connect(1, ConnectStep::Blocked);
    let gate = Arc::new(Notify::new());
    setup.script.gate_connect(1, Arc::clone(&gate));

    let mut tasks = JoinSet::new();
    let port = Arc::clone(&setup.built.port);
    tasks.spawn(async move { RefreshTrigger::refresh(port.as_ref()).await });
    wait_for_event(&setup.script, "connect:1").await;
    assert!(slot_url(&setup.slot).is_some());
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}

    assert_eq!(slot_url(&setup.slot), None);
    assert_eq!(setup.built.generation.observe().generation, 0);
}

#[tokio::test]
async fn callback_error_after_swap() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: Some(Arc::new(|| Err(FakeError::Callback))),
    })
    .await;
    let before = slot_url(&setup.slot);

    let outcome = RefreshTrigger::refresh(setup.built.port.as_ref()).await;

    assert!(matches!(
        outcome,
        Err(RefreshError::Replace {
            source: ReplacementError::Callback { .. }
        })
    ));
    assert_eq!(slot_url(&setup.slot), before);
    let observed = setup.built.generation.observe();
    assert_eq!(observed.generation, 1);
    assert_eq!(observed.handle.expect("the new generation is live").id, 1);
    assert_eq!(*lock(&setup.built.generation.last_refresh_time), None);
}

#[tokio::test]
async fn success_sets_timestamp() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);

    RefreshTrigger::refresh(setup.built.port.as_ref())
        .await
        .expect("the refresh succeeds");
    drop(guard);

    assert_eq!(
        *lock(&setup.built.generation.last_refresh_time),
        Some(utc("2024-01-01T00:00:00"))
    );
    assert_eq!(setup.built.generation.observe().generation, 1);
    assert!(logs.has("INFO", "[writer] RDS IAM token refreshed successfully."));
}

#[tokio::test]
async fn refresh_failure_keeps_pool() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    setup.script.script_connect(1, ConnectStep::Fail);

    let outcome = RefreshTrigger::refresh(setup.built.port.as_ref()).await;

    assert!(matches!(
        outcome,
        Err(RefreshError::Replace {
            source: ReplacementError::Connect { .. }
        })
    ));
    let observed = setup.built.generation.observe();
    let kept = observed.handle.expect("the old generation stays selected");
    assert!(Arc::ptr_eq(&kept, &setup.initial));
    assert!(observed.connected);
    assert_eq!(
        setup
            .initial
            .query("SELECT 1")
            .await
            .expect("the old pool still serves"),
        1
    );
}

#[tokio::test]
async fn reader_token_refresh() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[reader]".to_owned(),
        iam_endpoint: Some(IAMEndpoint {
            host: "replica.example".to_owned(),
            port: "6543".to_owned(),
            user: "reader".to_owned(),
            name: "replica".to_owned(),
            schema: Some("reporting".to_owned()),
        }),
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;

    RefreshTrigger::refresh(setup.built.port.as_ref())
        .await
        .expect("the explicit reader endpoint mints without writer fields");

    let endpoint = setup.mint.last_endpoint().expect("the minter was called");
    assert_eq!(endpoint.host, "replica.example");
    assert_eq!(endpoint.port, "6543");
    assert_eq!(endpoint.user, "reader");
    assert_eq!(endpoint.name, "replica");
    assert_eq!(endpoint.schema.as_deref(), Some("reporting"));
    assert_eq!(setup.built.generation.observe().generation, 1);
}

#[tokio::test]
async fn writer_reader_isolation() {
    let writer = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let reader = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[reader]".to_owned(),
        iam_endpoint: Some(IAMEndpoint {
            host: "replica.example".to_owned(),
            port: "5433".to_owned(),
            user: "reader".to_owned(),
            name: "litellm".to_owned(),
            schema: None,
        }),
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let writer_url = slot_url(&writer.slot);

    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    RefreshTrigger::refresh(reader.built.port.as_ref())
        .await
        .expect("the reader refreshes");
    drop(guard);

    assert_eq!(slot_url(&writer.slot), writer_url);
    assert_eq!(*lock(&writer.built.generation.last_refresh_time), None);
    assert!(lock(&reader.built.generation.last_refresh_time).is_some());
    assert_eq!(writer.built.generation.observe().generation, 0);
    assert_eq!(reader.built.generation.observe().generation, 1);
    assert!(logs.has("INFO", "[reader] RDS IAM token refreshed successfully."));
    assert_eq!(logs.count("[writer]"), 0);
    assert_eq!(writer.mint.endpoint_count(), 0);
}

#[tokio::test]
async fn writer_refresh_leaves_reader() {
    let writer = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[writer]".to_owned(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let reader = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: "[reader]".to_owned(),
        iam_endpoint: Some(IAMEndpoint {
            host: "replica.example".to_owned(),
            port: "5433".to_owned(),
            user: "reader".to_owned(),
            name: "litellm".to_owned(),
            schema: None,
        }),
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let reader_url = slot_url(&reader.slot);

    RefreshTrigger::refresh(writer.built.port.as_ref())
        .await
        .expect("the writer refreshes");

    assert_eq!(writer.built.generation.observe().generation, 1);
    assert_eq!(reader.built.generation.observe().generation, 0);
    assert_eq!(slot_url(&reader.slot), reader_url);
    assert_eq!(
        writer
            .mint
            .last_endpoint()
            .expect("the writer minter ran")
            .host,
        "db.example"
    );
    assert_eq!(reader.mint.endpoint_count(), 0);
}

#[test]
fn system_clock_reads_utc_now() {
    let clock = SystemClock;
    let before = Utc::now();
    let now = clock.now();
    let after = Utc::now();
    assert!(now >= before && now <= after);
}

#[test]
fn disabled_minter_returns_none() {
    let minter = DatabaseTokenMinter::<UnusedAzureFactory> { auth: None };
    let endpoint = IAMEndpoint {
        host: "db.example".to_owned(),
        port: "5432".to_owned(),
        user: "user".to_owned(),
        name: "litellm".to_owned(),
        schema: None,
    };
    assert!(
        minter
            .token(&endpoint)
            .expect("disabled minting succeeds")
            .is_none()
    );
}

#[test]
fn enabled_minter_returns_token() {
    let minter = DatabaseTokenMinter::<UnusedAzureFactory> {
        auth: Some(DatabaseTokenAuth::Rds(RdsIamTokenAuth {
            provider: Arc::new(FakeRdsProvider),
        })),
    };
    let endpoint = IAMEndpoint {
        host: "db.example".to_owned(),
        port: "5432".to_owned(),
        user: "user".to_owned(),
        name: "litellm".to_owned(),
        schema: None,
    };
    let token = minter
        .token(&endpoint)
        .expect("the token mints")
        .expect("a token is minted");
    assert_eq!(token.expose_secret(), "minted-token");
}

#[tokio::test]
async fn token_expired_reports_fresh_token() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    assert!(!RefreshTrigger::token_expired(setup.built.port.as_ref()));
}

#[tokio::test]
async fn token_expired_reports_missing_url() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: None,
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    assert!(RefreshTrigger::token_expired(setup.built.port.as_ref()));
}

#[tokio::test]
async fn minted_url_publishes_token() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_100)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let before = slot_url(&setup.slot);
    let minted = RefreshTrigger::minted_url(setup.built.port.as_ref())
        .expect("the token mints")
        .expect("a token is minted");
    assert!(!minted.expose_secret().is_empty());
    assert_eq!(
        slot_url(&setup.slot),
        Some(minted.expose_secret().to_owned())
    );
    assert_ne!(slot_url(&setup.slot), before);
}

#[test]
fn expired_only_after_expiry() {
    let token = token_url("X-Amz-Date=20240101T000000Z&X-Amz-Expires=1");
    let at_expiry = is_token_expired(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&token),
        now: naive("2024-01-01T00:00:01"),
    })
    .expect("the token parses");
    assert!(!at_expiry);
    let after_expiry = is_token_expired(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&token),
        now: naive("2024-01-01T00:00:02"),
    })
    .expect("the token parses");
    assert!(after_expiry);
}

#[test]
fn fresh_only_over_buffer() {
    let at_buffer = token_url("X-Amz-Date=20240101T000000Z&X-Amz-Expires=180");
    let not_fresh = token_refresh_not_needed(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&at_buffer),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    assert!(!not_fresh);
    let over_buffer = token_url("X-Amz-Date=20240101T000000Z&X-Amz-Expires=181");
    let fresh = token_refresh_not_needed(&RefreshQuery {
        kind: Some(Rds),
        token_url: Some(&over_buffer),
        now: naive("2024-01-01T00:00:00"),
    })
    .expect("the token parses");
    assert!(fresh);
}

#[tokio::test(start_paused = true)]
async fn stop_propagates_task_errors() {
    let setup = build_loop(LoopSpec {
        token_auth: Some(Rds),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token_url(EXPIRY_900)),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    let mut tasks = JoinSet::new();
    tasks.spawn(async { panic!("the refresh task crashed") });
    *lock(&setup.built.task) = Some(LoopTask {
        stop: Arc::new(Notify::new()),
        tasks,
    });
    let outcome = setup.built.stop().await;
    assert!(outcome.is_err());
    assert!(!setup.built.has_task());
}

#[tokio::test]
async fn out_of_range_entra_expiry_is_treated_as_expired() {
    let payload = URL_SAFE_NO_PAD.encode(r#"{"exp":253402300801}"#);
    let token = token_url(&format!("header.{payload}.signature"));
    let query = RefreshQuery {
        kind: Some(Azure),
        token_url: Some(&token),
        now: naive("2024-01-01T00:00:00"),
    };
    is_token_expired(&query).unwrap_err();
    let setup = build_loop(LoopSpec {
        token_auth: Some(Azure),
        log_prefix: String::new(),
        iam_endpoint: None,
        initial_url: Some(token.clone()),
        mint: MintOutcome::Token(EXPIRY_900.to_owned()),
        on_replaced: None,
    })
    .await;
    assert!(RefreshTrigger::token_expired(setup.built.port.as_ref()));
}
