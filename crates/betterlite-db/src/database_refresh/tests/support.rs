use std::collections::BTreeMap;
use std::io::{Result as IoResult, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Notify;
use tokio::task::{JoinSet, yield_now};
use tokio::time::{advance, timeout};
use tokio_postgres::NoTls;
use tracing::subscriber::set_default;
use tracing_subscriber::fmt::MakeWriter;

use crate::database_refresh::access::{self, RefreshFuture, RefreshTrigger};
use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::generation::{GenerationOptions, PoolGeneration};
use crate::database_refresh::handle::{
    PoolFuture, PoolHandle, PoolProvider, PoolTransport, TransactionControl, TransactionFuture,
};
use crate::database_refresh::replacement;

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn set_url(slot: &DatabaseUrlSlot, url: &str) {
    drop(slot.publish(Some(secret(url))));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum FakeError {
    #[error("connect failed")]
    Connect,
    #[error("query failed")]
    Query,
    #[error("mint failed")]
    Mint,
    #[error("callback failed")]
    Callback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectStep {
    Succeed,
    Fail,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QueryStep {
    Complete(u64),
    Fail,
    Blocked,
}

#[derive(Default)]
pub(super) struct FakeScript {
    events: Mutex<Vec<String>>,
    builds: Mutex<Vec<String>>,
    transports: Mutex<Vec<Option<usize>>>,
    timeouts: Mutex<Vec<Option<Duration>>>,
    connect_steps: Mutex<BTreeMap<usize, ConnectStep>>,
    connect_gates: Mutex<BTreeMap<usize, Arc<Notify>>>,
    query_steps: Mutex<BTreeMap<String, QueryStep>>,
    query_gates: Mutex<BTreeMap<String, Arc<Notify>>>,
    next_id: AtomicUsize,
}

impl FakeScript {
    pub(super) fn record(&self, event: String) {
        lock(&self.events).push(event);
    }

    #[must_use]
    pub(super) fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }

    #[must_use]
    pub(super) fn build_urls(&self) -> Vec<String> {
        lock(&self.builds).clone()
    }

    #[must_use]
    pub(super) fn transports(&self) -> Vec<Option<usize>> {
        lock(&self.transports).clone()
    }

    #[must_use]
    pub(super) fn connect_timeouts(&self) -> Vec<Option<Duration>> {
        lock(&self.timeouts).clone()
    }

    #[must_use]
    pub(super) fn has_event(&self, wanted: &str) -> bool {
        lock(&self.events).iter().any(|event| event == wanted)
    }

    #[must_use]
    pub(super) fn event_position(&self, wanted: &str) -> Option<usize> {
        lock(&self.events).iter().position(|event| event == wanted)
    }

    pub(super) fn script_connect(&self, id: usize, step: ConnectStep) {
        lock(&self.connect_steps).insert(id, step);
    }

    pub(super) fn gate_connect(&self, id: usize, gate: Arc<Notify>) {
        lock(&self.connect_gates).insert(id, gate);
    }

    pub(super) fn script_query(&self, sql: &str, step: QueryStep) {
        lock(&self.query_steps).insert(sql.to_owned(), step);
    }

    pub(super) fn gate_query(&self, sql: &str, gate: Arc<Notify>) {
        lock(&self.query_gates).insert(sql.to_owned(), gate);
    }

    pub(super) fn next_id(&self) -> usize {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn connect_step(&self, id: usize) -> ConnectStep {
        lock(&self.connect_steps)
            .get(&id)
            .copied()
            .unwrap_or(ConnectStep::Succeed)
    }

    fn connect_gate(&self, id: usize) -> Option<Arc<Notify>> {
        lock(&self.connect_gates).get(&id).cloned()
    }

    fn query_step(&self, sql: &str) -> QueryStep {
        lock(&self.query_steps)
            .get(sql)
            .copied()
            .unwrap_or(QueryStep::Complete(0))
    }

    fn query_gate(&self, sql: &str) -> Option<Arc<Notify>> {
        lock(&self.query_gates).get(sql).cloned()
    }
}

pub(super) struct FakeHandle {
    pub(super) id: usize,
    script: Arc<FakeScript>,
    connected: AtomicBool,
}

impl FakeHandle {
    async fn wait_for_gate(gate: Option<Arc<Notify>>) {
        if let Some(ready) = gate {
            ready.notified().await;
        }
    }
}

impl PoolHandle for FakeHandle {
    type Error = FakeError;

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.script.record(format!("connect:{}", self.id));
            lock(&self.script.timeouts).push(timeout);
            match self.script.connect_step(self.id) {
                ConnectStep::Succeed => {}
                ConnectStep::Fail => return Err(FakeError::Connect),
                ConnectStep::Blocked => {
                    Self::wait_for_gate(self.script.connect_gate(self.id)).await;
                }
            }
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn close(&self) {
        self.script.record(format!("close:{}", self.id));
        self.connected.store(false, Ordering::SeqCst);
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn query<'borrowed>(
        &'borrowed self,
        sql: &'borrowed str,
    ) -> PoolFuture<'borrowed, u64, Self::Error> {
        Box::pin(async move {
            self.script.record(format!("query:{sql}"));
            match self.script.query_step(sql) {
                QueryStep::Complete(rows) => Ok(rows),
                QueryStep::Fail => Err(FakeError::Query),
                QueryStep::Blocked => {
                    Self::wait_for_gate(self.script.query_gate(sql)).await;
                    Ok(0)
                }
            }
        })
    }
}

pub(super) struct FakeProvider {
    pub(super) script: Arc<FakeScript>,
}

impl PoolProvider for FakeProvider {
    type Handle = FakeHandle;
    type Error = FakeError;

    fn build(
        &self,
        url: &SecretString,
        transport: Option<&PoolTransport>,
    ) -> Result<FakeHandle, FakeError> {
        let id = self.script.next_id();
        self.script.record(format!("build:{id}"));
        lock(&self.script.builds).push(url.expose_secret().to_owned());
        lock(&self.script.transports).push(transport.and_then(|supplied| supplied.max_size));
        Ok(FakeHandle {
            id,
            script: Arc::clone(&self.script),
            connected: AtomicBool::new(false),
        })
    }
}

#[derive(Default)]
pub(super) struct FakeRefresh {
    pub(super) slot: DatabaseUrlSlot,
    pub(super) expired: AtomicBool,
    pub(super) refreshes: AtomicUsize,
    pub(super) expired_checks: AtomicUsize,
    pub(super) minted: Mutex<Option<SecretString>>,
    pub(super) mint_fails: AtomicBool,
    pub(super) gate: Mutex<Option<Arc<Notify>>>,
}

impl RefreshTrigger for FakeRefresh {
    type Error = FakeError;

    fn token_expired(&self) -> bool {
        self.expired_checks.fetch_add(1, Ordering::SeqCst);
        self.expired.load(Ordering::SeqCst)
    }

    fn refresh(&self) -> RefreshFuture<'_, Self::Error> {
        Box::pin(async move {
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            let gate = lock(&self.gate).clone();
            if let Some(ready) = gate {
                ready.notified().await;
            }
            Ok(())
        })
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Self::Error> {
        if self.mint_fails.load(Ordering::SeqCst) {
            return Err(FakeError::Mint);
        }
        lock(&self.minted).clone().map_or_else(
            || Ok(None),
            |url| {
                drop(self.slot.publish(Some(url.clone())));
                Ok(Some(url))
            },
        )
    }
}

pub(super) type FakeGeneration = PoolGeneration<FakeProvider, FakeRefresh, FakeError>;

pub(super) fn refresh() -> FakeRefresh {
    FakeRefresh {
        slot: DatabaseUrlSlot::default(),
        expired: AtomicBool::new(false),
        refreshes: AtomicUsize::new(0),
        expired_checks: AtomicUsize::new(0),
        minted: Mutex::new(None),
        mint_fails: AtomicBool::new(false),
        gate: Mutex::new(None),
    }
}

pub(super) fn options(slot: DatabaseUrlSlot) -> GenerationOptions {
    GenerationOptions {
        iam_token_db_auth: false,
        token_auth: None,
        db_url_env_var: "DATABASE_URL".to_owned(),
        url: slot,
        iam_endpoint: None,
        recreate_uses_datasource: false,
        log_prefix: String::new(),
    }
}

pub(super) fn token_options(slot: DatabaseUrlSlot) -> GenerationOptions {
    GenerationOptions {
        iam_token_db_auth: true,
        ..options(slot)
    }
}

pub(super) fn callback(
    script: &Arc<FakeScript>,
) -> Arc<dyn Fn() -> Result<(), FakeError> + Send + Sync> {
    let sink = Arc::clone(script);
    Arc::new(move || {
        sink.record("callback".to_owned());
        Ok(())
    })
}

pub(super) fn failing_callback() -> Arc<dyn Fn() -> Result<(), FakeError> + Send + Sync> {
    Arc::new(|| Err(FakeError::Callback))
}

pub(super) fn generation(
    script: &Arc<FakeScript>,
    options: GenerationOptions,
    refresh: Option<Arc<FakeRefresh>>,
    on_replaced: Option<Arc<dyn Fn() -> Result<(), FakeError> + Send + Sync>>,
) -> Arc<FakeGeneration> {
    let provider = Arc::new(FakeProvider {
        script: Arc::clone(script),
    });
    let initial = Arc::new(
        provider
            .build(&secret("postgres://initial"), None)
            .expect("the fake initial handle builds"),
    );
    Arc::new(
        FakeGeneration::new(
            Arc::clone(&provider),
            options,
            Some(initial),
            refresh,
            on_replaced,
        )
        .expect("the generation builds"),
    )
}

pub(super) async fn connected_generation(
    script: &Arc<FakeScript>,
    options: GenerationOptions,
    refresh: Option<Arc<FakeRefresh>>,
    on_replaced: Option<Arc<dyn Fn() -> Result<(), FakeError> + Send + Sync>>,
) -> Arc<FakeGeneration> {
    let built = generation(script, options, refresh, on_replaced);
    built
        .connect(None)
        .await
        .expect("the initial handle connects");
    built
}

pub(super) struct TestTransaction {
    pub(super) outcome: Option<Result<(), FakeError>>,
    pub(super) gate: Option<Arc<Notify>>,
    script: Arc<FakeScript>,
}

pub(super) fn transaction(script: &Arc<FakeScript>) -> TestTransaction {
    TestTransaction {
        outcome: None,
        gate: None,
        script: Arc::clone(script),
    }
}

impl TransactionControl for TestTransaction {
    type Error = FakeError;

    fn commit<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed,
    {
        Box::pin(self.finish("commit"))
    }

    fn rollback<'borrowed>(self) -> TransactionFuture<'borrowed, Self::Error>
    where
        Self: 'borrowed,
    {
        Box::pin(self.finish("rollback"))
    }
}

impl TestTransaction {
    async fn finish(self, action: &str) -> Result<(), FakeError> {
        self.script.record(action.to_owned());
        if let Some(gate) = self.gate {
            gate.notified().await;
        }
        self.outcome.unwrap_or(Ok(()))
    }
}

pub(super) async fn wait_for_event(script: &FakeScript, wanted: &str) {
    let reached = timeout(Duration::from_secs(5), async {
        while !script.has_event(wanted) {
            yield_now().await;
        }
    })
    .await;
    reached.expect("the scripted event happens");
}

pub(super) async fn stop<Item>(tasks: &mut JoinSet<Item>)
where
    Item: 'static,
{
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    #[must_use]
    pub(super) fn contents(&self) -> String {
        let bytes = lock(&self.0).clone();
        String::from_utf8(bytes).expect("the log writer records UTF-8")
    }
}

impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (CapturedLogs, impl tracing::Subscriber + Send + Sync) {
    let logs = CapturedLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn postgres_fixture() -> PostgresFixture {
    let configured = Environment::parse_from(["database-refresh-tests"])
        .url
        .map(SecretString::from);
    PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .expect("a Postgres fixture")
}

pub(super) fn fixture_pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().expect("the fixture URL parses");
    Pool::builder(Manager::new(config, NoTls))
        .max_size(1)
        .build()
        .expect("the fixture pool builds")
}

pub(super) async fn row_count(pool: &Pool, table: &str) -> i64 {
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .expect("the row count query runs");
    row.get(0)
}

pub(super) async fn close_fixture(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("fixture cleanup");
}

pub(super) const NEXT_URL: &str = "postgres://next";
pub(super) const BAD_URL: &str = "postgresql://betterlite:betterlite@127.0.0.1:1/betterlite";

pub(super) struct RetirementDeadline {
    pub(super) logs: String,
    pub(super) script: Arc<FakeScript>,
    pub(super) built: Arc<FakeGeneration>,
}

pub(super) async fn retirement_deadline() -> RetirementDeadline {
    let script = Arc::new(FakeScript::default());
    let slot = DatabaseUrlSlot::default();
    let built = connected_generation(&script, options(slot.clone()), None, None).await;
    let handle = access::operation(&built).expect("a connected operation");
    let tracker = handle.tracker.clone().expect("a tracker");
    let held = tracker.begin_transaction();
    set_url(&slot, NEXT_URL);
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    let lock_guard = built.replacement_lock.lock().await;
    let replaced = replacement::replace_locked(&built, &secret(NEXT_URL), None).await;
    drop(lock_guard);
    replaced.expect("the candidate replaces");
    yield_now().await;
    advance(Duration::from_secs(90)).await;
    yield_now().await;
    drop(held);
    yield_now().await;
    drop(guard);
    RetirementDeadline {
        logs: logs.contents(),
        script,
        built,
    }
}
