#![cfg(test)]

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_test_ports::refused_port::RefusedPort;
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool, PoolError, TimeoutType};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::error::SqlState;

use super::*;
use crate::autorouter_turn_sql::transaction::{AutoRouterTurnTransaction, TurnFlag, TurnTime};
use crate::autorouter_turn_sql::write::TurnWriteError;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::diagnostics::{DriverError, PoolFailure};
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;
use crate::tool_spend::RetryPause;

fn base() -> DateTime<FixedOffset> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .unwrap()
        .fixed_offset()
}

fn at(second: i64) -> TurnTime {
    TurnTime::Aware(
        base()
            .checked_add_signed(chrono::Duration::seconds(second))
            .unwrap(),
    )
}

fn micros(second: i64) -> i64 {
    turn_micros(at(second))
}

fn turn(api_key: &str, user_id: &str, session_id: &str, second: i64) -> AutoRouterTurnTransaction {
    AutoRouterTurnTransaction {
        api_key: api_key.to_owned(),
        session_id: session_id.to_owned(),
        router_name: "router".to_owned(),
        router_type: "simple-shuffle".to_owned(),
        model: "model".to_owned(),
        turn_at: at(second),
        total_tokens: 9,
        spend: Decimal::new(5, 1),
        saved_spend: Decimal::new(1, 1),
        classifier_cost: Decimal::new(1, 2),
        covered: TurnFlag(true),
        cache_hit: TurnFlag(false),
        cache_ttl_seconds: None,
        cache_touched: TurnFlag(false),
        tier: None,
        baseline_model: None,
        savings_estimated_turns: 0,
        savings_estimated_actual_spend: Decimal::ZERO,
        savings_estimated_saved_spend: Decimal::ZERO,
        user_id: user_id.to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Written {
    api_key: String,
    user_id: String,
    took_at: i64,
    user_scoped: bool,
}

fn written(api_key: &str, user_id: &str, second: i64, user_scoped: bool) -> Written {
    Written {
        api_key: api_key.to_owned(),
        user_id: user_id.to_owned(),
        took_at: micros(second),
        user_scoped,
    }
}

#[derive(Default)]
struct ScriptedWriter {
    script: Mutex<VecDeque<Option<TurnWriteError>>>,
    writes: Mutex<Vec<Written>>,
}

impl ScriptedWriter {
    fn with_script(steps: Vec<Option<TurnWriteError>>) -> Self {
        Self {
            script: Mutex::new(VecDeque::from(steps)),
            writes: Mutex::new(Vec::new()),
        }
    }

    fn succeeding() -> Self {
        Self::with_script(Vec::new())
    }

    fn writes(&self) -> Vec<Written> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next_failure(&self) -> Option<TurnWriteError> {
        self.script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .flatten()
    }
}

impl TurnWriter for ScriptedWriter {
    fn write_turn<'writer>(
        &'writer self,
        turn: &'writer AutoRouterTurnTransaction,
        user_scoped: bool,
    ) -> TurnFuture<'writer> {
        Box::pin(async move {
            self.writes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Written {
                    api_key: turn.api_key.clone(),
                    user_id: turn.user_id.clone(),
                    took_at: turn_micros(turn.turn_at),
                    user_scoped,
                });
            match self.next_failure() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }
}

struct FixedJitter {
    nanos: u64,
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        if upper_exclusive_nanos == 0 {
            return 0;
        }
        self.nanos.min(upper_exclusive_nanos.saturating_sub(1))
    }
}

#[derive(Default)]
struct RecordingPause {
    delays: Mutex<Vec<Duration>>,
}

impl RecordingPause {
    fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(std::future::ready(()))
    }
}

fn not_sent() -> TurnWriteError {
    TurnWriteError::Statement(DbError::NoDbConnected)
}

async fn refused_connection_error() -> tokio_postgres::Error {
    let refused = RefusedPort::bind().unwrap();
    let Err(error) = tokio_postgres::connect(
        &format!(
            "host=127.0.0.1 port={} user=missing",
            refused.address.port()
        ),
        tokio_postgres::NoTls,
    )
    .await
    else {
        panic!("connection unexpectedly succeeded");
    };
    error
}

async fn sqlstate_failure(code: &str) -> TurnWriteError {
    TurnWriteError::Statement(DbError::SqlState {
        code: SqlState::from_code(code),
        constraint: None,
        source: DriverError {
            driver: refused_connection_error().await,
        },
    })
}

async fn ambiguous_failure() -> TurnWriteError {
    TurnWriteError::Statement(DbError::Backend {
        source: DriverError {
            driver: refused_connection_error().await,
        },
    })
}

#[test]
fn partition_of_keeps_key_histories_apart_from_user_histories() {
    let keyed = partition_of(&turn("key-a", "user-a", "session", 1));
    let user = partition_of(&turn("", "user-a", "session", 1));
    let expected_key = partition::DrainPartition::Key {
        api_key: "key-a".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
    };
    let expected_user = partition::DrainPartition::User {
        user_id: "user-a".to_owned(),
        session_id: "session".to_owned(),
        router_name: "router".to_owned(),
    };
    assert!(keyed == expected_key);
    assert!(user == expected_user);
    assert!(keyed < user);
}

#[tokio::test]
async fn flush_orders_each_partition_by_turn_time() {
    let writer = ScriptedWriter::succeeding();
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-b", "user", "session", 5),
            turn("", "user-b", "session", 1),
            turn("key-a", "user", "session", 3),
            turn("key-a", "user", "session", 2),
            turn("key-b", "user-b", "session", 6),
            turn("key-b", "user-c", "session", 7),
        ])
        .await;
    assert_eq!(
        writer.writes(),
        vec![
            written("key-a", "user", 2, false),
            written("key-a", "user", 3, false),
            written("key-b", "user", 5, false),
            written("key-b", "user-b", 6, false),
            written("key-b", "user-c", 7, false),
            written("", "user-b", 1, false),
        ]
    );
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn a_proven_unapplied_write_is_retried_with_doubling_delays() {
    let writer = ScriptedWriter::with_script(vec![Some(not_sent()), Some(not_sent())]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![turn("key-a", "user-a", "session", 1)])
        .await;
    assert_eq!(
        writer.writes(),
        vec![
            written("key-a", "user-a", 1, false),
            written("key-a", "user-a", 1, false),
            written("key-a", "user-a", 1, false),
        ]
    );
    assert_eq!(
        pause.delays(),
        vec![Duration::from_secs(1), Duration::from_secs(2)]
    );
}

#[tokio::test]
async fn an_exhausted_retry_stops_the_partition() {
    let writer = ScriptedWriter::with_script(vec![Some(not_sent()), Some(not_sent())]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let mut rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup.retries = 1;
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session", 1),
            turn("key-a", "user-a", "session", 2),
        ])
        .await;
    assert_eq!(
        writer.writes(),
        vec![
            written("key-a", "user-a", 1, false),
            written("key-a", "user-a", 1, false),
        ]
    );
    assert_eq!(pause.delays(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn only_a_proven_unapplied_statement_is_replayed() {
    let refused = ambiguous_failure().await;
    let deadlock = sqlstate_failure("40P01").await;
    let check_violation = sqlstate_failure("23514").await;
    assert!(safe_to_replay(&not_sent()));
    assert!(safe_to_replay(&TurnWriteError::Statement(DbError::Pool {
        source: PoolFailure {
            driver: PoolError::Timeout(TimeoutType::Wait),
        },
    })));
    assert!(!safe_to_replay(&refused));
    assert!(!safe_to_replay(&deadlock));
    assert!(!safe_to_replay(&check_violation));
    assert!(!safe_to_replay(&TurnWriteError::Statement(
        DbError::CreateNull
    )));
}

#[tokio::test]
async fn a_deadlock_is_never_replayed_and_stops_the_partition() {
    let writer = ScriptedWriter::with_script(vec![Some(sqlstate_failure("40P01").await)]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session", 1),
            turn("key-a", "user-a", "session", 2),
        ])
        .await;
    assert_eq!(writer.writes(), vec![written("key-a", "user-a", 1, false)]);
    assert!(pause.delays().is_empty());
}

#[tokio::test]
async fn a_rejected_turn_stops_its_partition_and_other_partitions_advance() {
    let writer = ScriptedWriter::with_script(vec![Some(sqlstate_failure("23514").await)]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session", 1),
            turn("key-a", "user-a", "session", 2),
            turn("key-b", "user-b", "session", 3),
        ])
        .await;
    assert_eq!(
        writer.writes(),
        vec![
            written("key-a", "user-a", 1, false),
            written("key-b", "user-b", 3, false),
        ]
    );
}

#[tokio::test]
async fn a_failed_key_partition_advances_other_users_independently() {
    let writer = ScriptedWriter::with_script(vec![
        Some(TurnWriteError::Statement(DbError::CreateNull)),
        Some(TurnWriteError::Statement(DbError::CreateNull)),
    ]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session", 1),
            turn("key-a", "user-a", "session", 2),
            turn("key-a", "user-c", "session", 3),
            turn("key-a", "user-b", "session", 4),
            turn("key-a", "", "session", 5),
            turn("key-a", "user-b", "session", 6),
        ])
        .await;
    assert_eq!(
        writer.writes(),
        vec![
            written("key-a", "user-a", 1, false),
            written("key-a", "user-b", 4, true),
            written("key-a", "user-c", 3, true),
        ]
    );
}

#[tokio::test]
async fn a_failed_user_partition_recovers_nothing() {
    let writer =
        ScriptedWriter::with_script(vec![Some(TurnWriteError::Statement(DbError::CreateNull))]);
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("", "user-a", "session", 1),
            turn("", "user-a", "session", 2),
        ])
        .await;
    assert_eq!(writer.writes(), vec![written("", "user-a", 1, false)]);
}

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["autorouter-drain-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the migration text");
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .expect("the database fixture");
    let config = fixture
        .options
        .url
        .expose_secret()
        .parse()
        .expect("the database url");
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(2)
        .build()
        .expect("the database pool");
    let store = PostgresStore { pool: Some(pool) };
    (fixture, store)
}

async fn close(fixture: PostgresFixture, store: PostgresStore) {
    if let Some(pool) = store.pool {
        pool.close();
    }
    fixture.close_async().await.expect("the fixture teardown");
}

#[tokio::test]
async fn postgres_flush_advances_both_session_scopes() {
    let (fixture, store) = database().await;
    let writer = PostgresTurns {
        store: store.clone(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session-1", 1),
            turn("", "user-b", "session-1", 2),
        ])
        .await;
    let session = store.session().await.expect("the rollup session");
    let keyed = session
        .query(
            "SELECT api_key, turns FROM \"LiteLLM_AutoRouterSession\"",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, i32>(1)))
        .collect::<Vec<_>>();
    assert_eq!(keyed, vec![("key-a".to_owned(), 1_i32)]);
    let users = session
        .query(
            "SELECT user_id, api_key, turns FROM \"LiteLLM_AutoRouterUserSession\" ORDER BY user_id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get::<_, String>(0),
                row.get::<_, String>(1),
                row.get::<_, i32>(2),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        users,
        vec![
            ("user-a".to_owned(), "key-a".to_owned(), 1_i32),
            ("user-b".to_owned(), String::new(), 1_i32),
        ]
    );
    drop(session);
    close(fixture, store).await;
}

#[tokio::test]
async fn postgres_failed_key_partition_still_writes_other_user_histories() {
    let (fixture, store) = database().await;
    let probe = store.session().await.expect("the rollup session");
    probe
        .batch_execute("ALTER TABLE \"LiteLLM_AutoRouterSession\" ADD CHECK (api_key <> 'key-a')")
        .await
        .unwrap();
    let writer = PostgresTurns {
        store: store.clone(),
    };
    let jitter = FixedJitter { nanos: 0 };
    let pause = RecordingPause::default();
    let rollup = AutoRouterDrain::new(&writer, &jitter, &pause);
    rollup
        .flush_turn_transactions(vec![
            turn("key-a", "user-a", "session-1", 1),
            turn("key-a", "user-a", "session-1", 2),
            turn("key-a", "user-b", "session-1", 3),
            turn("key-a", "user-c", "session-1", 4),
        ])
        .await;
    let keyed: i64 = probe
        .query_one("SELECT COUNT(*) FROM \"LiteLLM_AutoRouterSession\"", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(keyed, 0);
    let users = probe
        .query(
            "SELECT user_id, api_key, turns FROM \"LiteLLM_AutoRouterUserSession\" ORDER BY user_id",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get::<_, String>(0),
                row.get::<_, String>(1),
                row.get::<_, i32>(2),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        users,
        vec![
            ("user-b".to_owned(), "key-a".to_owned(), 1_i32),
            ("user-c".to_owned(), "key-a".to_owned(), 1_i32),
        ]
    );
    drop(probe);
    close(fixture, store).await;
}
