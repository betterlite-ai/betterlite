use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, NaiveDateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::tls::NoTlsStream;
use tokio_postgres::{Client, Connection, NoTls, Socket};

use crate::budget_reset_policy::cache_invalidation::{CacheDeleteFuture, ResetCache};
use crate::entity_spend_commit::EntityTableWriter;
use crate::entity_spend_commit::balances::PostgresEntityTables;
use crate::entity_spend_commit::progress::{EntityCommitError, EntitySpendCommit};
use crate::key_user_writes::retry::WriteClock;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::migration_ledger::lock::JitterSource;
use crate::postgres_store::session::PostgresStore;
use crate::spend_commit::policy::SpendRetryPolicy;
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions};
use crate::spend_commit::transaction::SpendDeadline;
use crate::spend_log_storage::rows::writer::{SpendLogFailureEvent, SpendLogFailureSink};
use crate::tool_spend::RetryPause;

pub(crate) struct FixedClock {
    pub(crate) instant: Instant,
}

impl WriteClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        flush_moment().and_utc()
    }

    fn instant(&self) -> Instant {
        self.instant
    }
}

// The last_active every key written through FixedClock carries.
pub(crate) fn flush_moment() -> NaiveDateTime {
    flush_moment_tick(0)
}

// What a clock that reads a millisecond later each time returns on its tick-th read.
pub(crate) fn flush_moment_tick(tick: u32) -> NaiveDateTime {
    let nanos = 123_000_000_u32.saturating_add(tick.saturating_mul(1_000_000));
    DateTime::<Utc>::from_timestamp(1_767_225_600, nanos)
        .unwrap()
        .naive_utc()
}

// A clock whose every read is one millisecond later than the last, so a stamp read once per table is distinguishable from one read per row.
pub(crate) struct TickingClock {
    pub(crate) reads: AtomicU32,
}

impl WriteClock for TickingClock {
    fn now(&self) -> DateTime<Utc> {
        let tick = self.reads.fetch_add(1, Ordering::SeqCst);
        flush_moment_tick(tick).and_utc()
    }

    fn instant(&self) -> Instant {
        Instant::now()
    }
}

pub(crate) struct NoJitter;

impl JitterSource for NoJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

// Retry waits are recorded and skipped.
#[derive(Default)]
pub(crate) struct PauseLog {
    waits: Mutex<Vec<Duration>>,
}

impl PauseLog {
    pub(crate) fn delays(&self) -> Vec<Duration> {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RetryPause for PauseLog {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.waits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
        Box::pin(ready(()))
    }
}

#[derive(Default)]
pub(crate) struct FailureLog {
    events: Mutex<Vec<SpendLogFailureEvent>>,
}

impl FailureLog {
    pub(crate) fn count(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl SpendLogFailureSink for FailureLog {
    fn emit(&self, event: SpendLogFailureEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

// One ordered log the writer, the progress callback and the cache all append to.
#[derive(Default)]
pub(crate) struct Events {
    entries: Mutex<Vec<String>>,
}

impl Events {
    pub(crate) fn push(&self, entry: String) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the cache is down")]
pub(crate) struct CacheDown;

pub(crate) struct RecordingCache<'events> {
    pub(crate) events: &'events Events,
    pub(crate) failing: bool,
}

impl ResetCache for RecordingCache<'_> {
    type Failure = CacheDown;

    fn delete_spend_counters<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheDown> {
        self.events.push(format!("counters {}", keys.join(",")));
        Box::pin(ready(Ok(())))
    }

    fn delete_user_api_keys<'cache>(
        &'cache self,
        keys: &'cache [String],
    ) -> CacheDeleteFuture<'cache, CacheDown> {
        self.events.push(format!("evict {}", keys.join(",")));
        let outcome = if self.failing { Err(CacheDown) } else { Ok(()) };
        Box::pin(ready(outcome))
    }
}

pub(crate) struct Ports {
    pub(crate) clock: FixedClock,
    pub(crate) jitter: NoJitter,
    pub(crate) pause: PauseLog,
    pub(crate) failures: FailureLog,
    pub(crate) events: Events,
}

impl Ports {
    pub(crate) fn new() -> Self {
        Self {
            clock: FixedClock {
                instant: Instant::now(),
            },
            jitter: NoJitter,
            pause: PauseLog::default(),
            failures: FailureLog::default(),
            events: Events::default(),
        }
    }

    pub(crate) fn policy(&self, retries: u32) -> SpendRetryPolicy<'_> {
        SpendRetryPolicy {
            retries,
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            failures: &self.failures,
        }
    }

    pub(crate) const fn cache(&self) -> RecordingCache<'_> {
        RecordingCache {
            events: &self.events,
            failing: false,
        }
    }

    pub(crate) fn tables(&self, store: PostgresStore) -> PostgresEntityTables<'_> {
        PostgresEntityTables {
            store,
            clock: &self.clock,
            deadline: SpendDeadline::SOURCE,
        }
    }
}

pub(crate) type CommitOutcome<Writer> =
    Result<(), EntityCommitError<<Writer as EntityTableWriter>::Error, CacheDown>>;

// Runs one commit with two retries and returns the tables it marked committed, in the order it marked them.
pub(crate) async fn commit_through<Writer>(
    ports: &Ports,
    writer: &Writer,
    cache: &RecordingCache<'_>,
    transactions: &SpendTableTransactions,
) -> (Vec<SpendTable>, CommitOutcome<Writer>)
where
    Writer: EntityTableWriter,
{
    let commit = EntitySpendCommit {
        writer,
        policy: ports.policy(2),
        cache,
    };
    let mut committed = Vec::new();
    let outcome = commit
        .commit_spend_updates(transactions, |table| {
            ports.events.push(format!("committed {}", table.name()));
            committed.push(table);
        })
        .await;
    (committed, outcome)
}

pub(crate) fn increments(rows: &[(&str, &str)]) -> BTreeMap<String, Decimal> {
    rows.iter()
        .map(|(id, cost)| ((*id).to_owned(), Decimal::from_str(cost).unwrap()))
        .collect()
}

pub(crate) fn transactions(tables: &[(SpendTable, &[(&str, &str)])]) -> SpendTableTransactions {
    tables
        .iter()
        .map(|(table, rows)| (*table, Some(increments(rows))))
        .collect()
}

// One existing row for every entity table, each holding spend 1 and, where it has one, total_spend 3.
pub(crate) const SEED_SQL: &str = r#"
INSERT INTO "LiteLLM_BudgetTable" (budget_id, created_by, updated_by) VALUES ('b1', 'test', 'test');
INSERT INTO "LiteLLM_UserTable" (user_id, spend) VALUES ('u1', 1), ('u 1', 1);
INSERT INTO "LiteLLM_EndUserTable" (user_id, spend) VALUES ('e1', 1);
INSERT INTO "LiteLLM_VerificationToken" (token, spend, total_spend) VALUES ('k1', 1, 3);
INSERT INTO "LiteLLM_TeamTable" (team_id, spend, members_with_roles) VALUES ('t1', 1, '[{"user_id": "u1", "role": "user"}]');
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, spend, total_spend) VALUES ('u1', 't1', 1, 3);
INSERT INTO "LiteLLM_OrganizationTable" (organization_id, organization_alias, budget_id, created_by, updated_by, spend) VALUES ('o 1', 'org', 'b1', 'test', 'test', 1);
INSERT INTO "LiteLLM_OrganizationMembership" (user_id, organization_id, spend) VALUES ('u 1', 'o 1', 1);
INSERT INTO "LiteLLM_ProjectTable" (project_id, created_by, updated_by, spend) VALUES ('p1', 'test', 'test', 1);
INSERT INTO "LiteLLM_TagTable" (tag_name, spend) VALUES ('tag1', 1);
INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" (access_group_name, spend) VALUES ('g1', 1);
INSERT INTO "LiteLLM_AgentsTable" (agent_id, agent_name, agent_card_params, created_by, updated_by, spend) VALUES ('a1', 'agent', '{}', 'test', 'test', 1);
"#;

// Spend of one team membership, looked up by "team/user".
pub(crate) const MEMBER_SPEND: &str =
    r#"SELECT spend::numeric FROM "LiteLLM_TeamMembership" WHERE team_id || '/' || user_id = $1"#;
pub(crate) const MEMBER_TOTAL_SPEND: &str = r#"SELECT total_spend::numeric FROM "LiteLLM_TeamMembership" WHERE team_id || '/' || user_id = $1"#;

pub(crate) fn amount(text: &str) -> Option<Decimal> {
    Some(Decimal::from_str(text).unwrap())
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    pub(crate) client: Client,
    drivers: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl Database {
    pub(crate) async fn open(connections: usize) -> Self {
        let url = Environment::parse_from(["entity-spend-commit"])
            .url
            .map(SecretString::from);
        let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
        let fixture = PostgresFixture::provision_with_statements(url.as_ref(), statements)
            .resolve()
            .await
            .expect("a test database");
        let manager = Manager::new(fixture.options.config.clone(), NoTls);
        let pool = Pool::builder(manager)
            .max_size(connections)
            .build()
            .unwrap();
        let (client, connection) = connect(&fixture).await;
        let mut drivers = JoinSet::new();
        drivers.spawn(connection);
        Self {
            fixture,
            store: PostgresStore { pool: Some(pool) },
            client,
            drivers,
        }
    }

    // A second session of its own, for holding locks and rows while the commit runs.
    pub(crate) async fn session(&mut self) -> Client {
        let (client, connection) = connect(&self.fixture).await;
        self.drivers.spawn(connection);
        client
    }

    pub(crate) async fn seed(&self, statements: &str) {
        self.client.batch_execute(statements).await.unwrap();
    }

    pub(crate) async fn spend(&self, sql: &str, id: &str) -> Option<Decimal> {
        self.client
            .query_opt(sql, &[&id])
            .await
            .unwrap()
            .map(|row| row.get(0))
    }

    pub(crate) async fn close(self) {
        let Self {
            fixture,
            store,
            client,
            drivers,
        } = self;
        drop(client);
        drop(drivers);
        if let Some(pool) = store.pool {
            pool.close();
        }
        fixture.close_async().await.unwrap();
    }
}

async fn connect(fixture: &PostgresFixture) -> (Client, Connection<Socket, NoTlsStream>) {
    tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
        .await
        .unwrap()
}
