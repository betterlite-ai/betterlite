use std::future::ready;
use std::sync::Arc;
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use deadpool_postgres::Pool;
use secrecy::{ExposeSecret as _, SecretString};

use crate::database_refresh::access;
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::generation::PoolGeneration;
use crate::database_refresh::handle::{
    OperationHandle, PoolFuture, PoolHandle, PoolProvider, PoolTransport, TransactionEnd,
};
use crate::database_refresh::native::{NativePool, NativePoolError, NativePoolProvider};
use crate::database_refresh::replacement::{self, ReplacementError};

use super::support::{
    BAD_URL, FakeError, FakeRefresh, FakeScript, close_fixture, fixture_pool, options,
    postgres_fixture, row_count, secret, set_url,
};

struct ObservedProvider {
    script: Arc<FakeScript>,
    inner: NativePoolProvider,
}

impl ObservedProvider {
    fn new(script: Arc<FakeScript>) -> Self {
        Self {
            script,
            inner: NativePoolProvider,
        }
    }
}

struct ObservedHandle {
    id: usize,
    pool: NativePool,
    script: Arc<FakeScript>,
}

impl ObservedHandle {
    async fn count(&self, table: &str) -> i64 {
        let client = self.pool.session().await.expect("a pool session");
        let row = client
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .await
            .expect("the row count query runs");
        row.get(0)
    }
}

impl PoolProvider for ObservedProvider {
    type Handle = ObservedHandle;
    type Error = NativePoolError;

    fn build(
        &self,
        url: &SecretString,
        transport: Option<&PoolTransport>,
    ) -> Result<ObservedHandle, NativePoolError> {
        let id = self.script.next_id();
        self.script.record(format!("build:{id}"));
        let pool = self.inner.build(url, transport)?;
        Ok(ObservedHandle {
            id,
            pool,
            script: Arc::clone(&self.script),
        })
    }
}

impl PoolHandle for ObservedHandle {
    type Error = NativePoolError;

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.script.record(format!("connect:{}", self.id));
            self.pool.connect(timeout).await
        })
    }

    fn close(&self) {
        self.script.record(format!("close:{}", self.id));
        self.pool.close();
    }

    fn is_connected(&self) -> bool {
        self.pool.is_connected()
    }

    fn query<'borrowed>(
        &'borrowed self,
        sql: &'borrowed str,
    ) -> PoolFuture<'borrowed, u64, Self::Error> {
        Box::pin(async move { self.pool.query(sql).await })
    }
}

type ObservedGeneration = PoolGeneration<ObservedProvider, FakeRefresh, FakeError>;

struct ObservedSetup {
    fixture: PostgresFixture,
    pool: Pool,
    script: Arc<FakeScript>,
    initial: Arc<ObservedHandle>,
    built: Arc<ObservedGeneration>,
}

async fn observed_generation() -> ObservedSetup {
    let fixture = postgres_fixture().await;
    let good = fixture.options.url.clone();
    let pool = fixture_pool(&good);
    let script = Arc::new(FakeScript::default());
    let provider = Arc::new(ObservedProvider::new(Arc::clone(&script)));
    let initial = Arc::new(
        provider
            .build(&good, None)
            .expect("the initial native pool builds"),
    );
    let slot = DatabaseUrlSlot::default();
    set_url(&slot, good.expose_secret());
    let built: Arc<ObservedGeneration> = Arc::new(
        PoolGeneration::new(
            Arc::clone(&provider),
            options(slot),
            Some(Arc::clone(&initial)),
            None,
            None,
        )
        .expect("the generation builds"),
    );
    built
        .connect(None)
        .await
        .expect("the initial pool connects");
    ObservedSetup {
        fixture,
        pool,
        script,
        initial,
        built,
    }
}

async fn commit_row(built: &ObservedGeneration, value: &str) {
    let operations = access::operation(built).expect("a connected operation");
    operations
        .query("CREATE TABLE refresh_rows (note text)")
        .await
        .expect("the table is created");
    operations
        .query(&format!("INSERT INTO refresh_rows VALUES ('{value}')"))
        .await
        .expect("the row commits");
}

#[tokio::test]
async fn postgres_recovery_stops_old_before_connect() {
    let setup = observed_generation().await;
    commit_row(&setup.built, "old").await;
    set_url(&setup.built.options.url, BAD_URL);
    let outcome = replacement::recreate(&setup.built, &secret(BAD_URL), None, None).await;
    assert!(matches!(outcome, Err(ReplacementError::Connect { .. })));
    let close = setup
        .script
        .event_position("close:0")
        .expect("the old pool closed");
    let build_at = setup
        .script
        .event_position("build:1")
        .expect("the replacement built");
    let connect_at = setup
        .script
        .event_position("connect:1")
        .expect("the replacement connected");
    assert!(close < build_at);
    assert!(build_at < connect_at);
    let observed = setup.built.observe();
    assert_eq!(observed.generation, 0);
    assert_eq!(observed.handle.expect("the replacement is selected").id, 1);
    assert!(!observed.connected);
    assert!(!setup.initial.is_connected());
    setup.initial.query("SELECT 1").await.unwrap_err();
    assert_eq!(row_count(&setup.pool, "refresh_rows").await, 1);
    let ObservedSetup { fixture, pool, .. } = setup;
    close_fixture(fixture, pool).await;
}

#[tokio::test]
async fn postgres_refresh_candidate_failure_preserves_old() {
    let setup = observed_generation().await;
    commit_row(&setup.built, "kept").await;
    set_url(&setup.built.options.url, BAD_URL);
    let lock_guard = setup.built.replacement_lock.lock().await;
    let outcome = replacement::replace_locked(&setup.built, &secret(BAD_URL), None).await;
    drop(lock_guard);
    assert!(matches!(outcome, Err(ReplacementError::Connect { .. })));
    let observed = setup.built.observe();
    assert_eq!(observed.generation, 0);
    assert!(Arc::ptr_eq(
        &observed.handle.expect("the old handle stays selected"),
        &setup.initial
    ));
    assert!(observed.connected);
    assert!(!setup.script.has_event("close:0"));
    assert!(setup.script.has_event("close:1"));
    assert_eq!(setup.initial.count("refresh_rows").await, 1);
    assert_eq!(row_count(&setup.pool, "refresh_rows").await, 1);
    let ObservedSetup { fixture, pool, .. } = setup;
    close_fixture(fixture, pool).await;
}

async fn run_transaction(
    setup: &ObservedSetup,
    operations: &OperationHandle<ObservedHandle>,
    value: &str,
    end: TransactionEnd,
) {
    let mut client = setup.initial.pool.session().await.expect("a session");
    let session = client.transaction().await.expect("a transaction begins");
    let tracked = operations
        .begin_transaction(ready(Ok::<_, tokio_postgres::Error>(session)))
        .await
        .expect("a tracked transaction");
    tracked
        .value
        .execute(&format!("INSERT INTO refresh_rows VALUES ('{value}')"), &[])
        .await
        .expect("the insert runs inside the transaction");
    operations
        .finish_transaction(tracked, end)
        .await
        .expect("the transaction finishes");
}

#[tokio::test]
async fn postgres_transaction_commit_and_rollback() {
    let setup = observed_generation().await;
    let operations = access::operation(&setup.built).expect("a connected operation");
    let tracker = operations.tracker.clone().expect("a tracker");
    operations
        .query("CREATE TABLE refresh_rows (note text)")
        .await
        .expect("the table is created");
    run_transaction(&setup, &operations, "committed", TransactionEnd::Commit).await;
    assert!(tracker.status().drained);
    assert_eq!(setup.initial.count("refresh_rows").await, 1);
    run_transaction(&setup, &operations, "rolled back", TransactionEnd::Rollback).await;
    assert!(tracker.status().drained);
    assert_eq!(setup.initial.count("refresh_rows").await, 1);
    let ObservedSetup { fixture, pool, .. } = setup;
    close_fixture(fixture, pool).await;
}
