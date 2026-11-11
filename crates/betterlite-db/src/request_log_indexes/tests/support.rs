use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending, ready};
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_postgres::Client as PgClient;
use tokio_postgres::NoTls;
use tokio_postgres::types::{FromSqlOwned, ToSql};
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use url::Url;
use uuid::Uuid;

use crate::migration_ledger::lock::JitterSource;
use crate::request_log_index_catalog::catalog::{EquivalentIndex, IndexState, Relation, TableKind};
use crate::request_log_index_catalog::definitions::{REQUEST_LOG_INDEXES, RequestLogIndex};
use crate::request_log_indexes::connect;
use crate::request_log_indexes::database::{
    DatabaseFailure, DatabaseFuture, IndexDatabase, IndexRef, PostgresDatabase,
};
use crate::request_log_indexes::ensure::ensure_indexes;
use crate::request_log_indexes::leaf::build_leaf;
use crate::request_log_indexes::locks::RetryPause;
use crate::request_log_indexes::partition::{PartitionTarget, build_on_partitioned_table};
use crate::request_log_indexes::{IndexBuilder, IndexDependencies, RequestLogIndexError};

#[derive(Parser)]
struct TestEnvironment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn postgres_test_url() -> Option<SecretString> {
    TestEnvironment::parse_from(["request-log-indexes-tests"])
        .url
        .map(SecretString::from)
}

const TEST_SCHEMA: &str = "request_log_indexes";

pub(super) struct PostgresTest {
    pub(super) fixture: PostgresFixture,
    pub(super) schema: String,
    pub(super) url: SecretString,
    pub(super) client: tokio_postgres::Client,
    drivers: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl PostgresTest {
    // Each test owns a database: the coordinator lock is per-database, so parallel tests never share it
    pub(super) async fn new() -> Self {
        let mut generator = RandomDatabaseName;
        let mut task = PostgresFixture::provision_with_name_generator(
            postgres_test_url().as_ref(),
            &FixtureSeed::default(),
            &mut generator,
        );
        let fixture = task.resolve().await.unwrap();
        create_test_schema(&fixture).await;
        let url = with_test_schema(&fixture.options.url, TEST_SCHEMA);
        let mut drivers = JoinSet::new();
        let (client, connection) = tokio_postgres::connect(url.expose_secret(), NoTls)
            .await
            .unwrap();
        drivers.spawn(connection);
        Self {
            fixture,
            schema: TEST_SCHEMA.to_owned(),
            url,
            client,
            drivers,
        }
    }

    pub(super) async fn service(&self) -> PostgresDatabase {
        connect::connect(&self.url, connect::CONNECT_TIMEOUT)
            .await
            .unwrap()
    }

    pub(super) async fn another_client(
        &self,
    ) -> (
        tokio_postgres::Client,
        JoinSet<Result<(), tokio_postgres::Error>>,
    ) {
        let (client, connection) = tokio_postgres::connect(self.url.expose_secret(), NoTls)
            .await
            .unwrap();
        let mut drivers = JoinSet::new();
        drivers.spawn(connection);
        (client, drivers)
    }

    pub(super) fn finish(self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let Self {
                fixture,
                client,
                mut drivers,
                ..
            } = self;
            // Connections a test left open would block the fixture's DROP DATABASE
            terminate_other_backends(&client).await;
            drop(client);
            drivers.shutdown().await;
            fixture.close_async().await.unwrap();
        })
    }
}

async fn terminate_other_backends(client: &PgClient) {
    for _attempt in 0_u32..100_u32 {
        let remaining: i64 = client
            .query_one(
                "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
                &[],
            )
            .await
            .unwrap()
            .get(0);
        if remaining == 0 {
            return;
        }
        client
            .batch_execute(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()",
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(10)).await;
    }
}

async fn create_test_schema(fixture: &PostgresFixture) {
    let (client, connection) = tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut drivers = JoinSet::new();
    drivers.spawn(connection);
    client
        .batch_execute(&format!("CREATE SCHEMA \"{TEST_SCHEMA}\""))
        .await
        .unwrap();
    drop(client);
    drivers.shutdown().await;
}

fn with_test_schema(url: &SecretString, schema: &str) -> SecretString {
    let mut parsed = Url::parse(url.expose_secret()).unwrap();
    let parameters: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "options")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(parameters)
        .append_pair("options", &format!("-csearch_path={schema}"));
    let query = parsed.query().unwrap_or_default().replace('+', "%20");
    parsed.set_query(Some(&query));
    SecretString::from(parsed.to_string())
}

// Wraps the real database and records the DDL a test asserts the order of
pub(super) struct RecordingDatabase {
    pub(super) inner: PostgresDatabase,
    pub(super) statements: Arc<Mutex<Vec<String>>>,
}

impl RecordingDatabase {
    pub(super) fn new(inner: PostgresDatabase) -> Self {
        Self {
            inner,
            statements: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn statements(&self) -> Vec<String> {
        self.statements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl IndexDatabase for RecordingDatabase {
    type Error = RequestLogIndexError;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error> {
        self.inner.table_kind(schema, table)
    }

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error> {
        self.inner.index_state(schema, index)
    }

    fn equivalent_indexes<'database>(
        &'database self,
        target: IndexRef<'database>,
        definition: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        self.inner.equivalent_indexes(target, definition)
    }

    fn children_without_the_index<'database>(
        &'database self,
        target: IndexRef<'database>,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        self.inner.children_without_the_index(target)
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        self.statements
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(statement.to_owned());
        self.inner.execute(statement)
    }

    fn coordinate<'database>(
        &'database self,
        step: DatabaseFuture<'database, bool, Self::Error>,
    ) -> DatabaseFuture<'database, bool, Self::Error> {
        self.inner.coordinate(step)
    }
}

pub(super) const API_KEY_INDEX: &str = "LiteLLM_SpendLogs_api_key_startTime_idx";
pub(super) const CALL_ID_INDEX: &str = "LiteLLM_SpendLogs_litellm_call_id_idx";
pub(super) const SPEND_LOGS: &str = "LiteLLM_SpendLogs";

pub(super) fn index_by_name(name: &str) -> RequestLogIndex<'_> {
    REQUEST_LOG_INDEXES
        .iter()
        .copied()
        .find(|index| index.name == name)
        .unwrap()
}

const INDEX_STATE_SQL: &str = "SELECT t.relname AS \"table\", x.indisvalid AS valid FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid JOIN pg_class t ON t.oid = x.indrelid WHERE i.relname = $1 AND i.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())";
const OID_SQL: &str = "SELECT c.oid FROM pg_class c WHERE c.relname = $1 AND c.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())";
const OID_IN_SCHEMA_SQL: &str = "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2";
const RELKIND_SQL: &str = "SELECT relkind::text FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())";
const REPLICA_SQL: &str = "SELECT relreplident::text FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())";
const COLUMNS_SQL: &str = "SELECT a.attname FROM pg_index x JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = ANY (x.indkey::int2[]) WHERE x.indexrelid = (SELECT oid FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema()))";
const CHILDREN_SQL: &str = "SELECT child.relname FROM pg_inherits i JOIN pg_class parent ON parent.oid = i.inhparent JOIN pg_class child ON child.oid = i.inhrelid WHERE parent.relname = $1 AND parent.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = $2) ORDER BY child.relname";

pub(super) async fn create_spend_logs(test: &PostgresTest) {
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"{SPEND_LOGS}\" (request_id text, api_key text, \"startTime\" timestamptz, litellm_call_id text);"
        ))
        .await
        .unwrap();
}

pub(super) async fn duplicate_api_key_rows(client: &PgClient, table: &str) {
    client.batch_execute(&format!("INSERT INTO \"{table}\" (request_id, api_key, \"startTime\", litellm_call_id) VALUES ('one', 'same', '2026-01-01T00:00:00Z', 'call-one'), ('two', 'same', '2026-01-01T00:00:00Z', 'call-two');")).await.unwrap();
}

// A UNIQUE CONCURRENTLY build over duplicate rows fails and leaves the named index invalid
pub(super) async fn leave_invalid_index(client: &PgClient, name: &str, table: &str, columns: &str) {
    let statement =
        format!("CREATE UNIQUE INDEX CONCURRENTLY \"{name}\" ON \"{table}\" ({columns})");
    client.batch_execute(&statement).await.unwrap_err();
}

pub(super) async fn index_state_of(client: &PgClient, name: &str) -> Option<(String, bool)> {
    let row = client.query_opt(INDEX_STATE_SQL, &[&name]).await.unwrap()?;
    Some((row.get("table"), row.get("valid")))
}

pub(super) async fn index_oid(client: &PgClient, name: &str) -> Option<u32> {
    optional_value(client, OID_SQL, &[&name]).await
}

pub(super) async fn relation_kind(client: &PgClient, name: &str) -> String {
    first_value(client, RELKIND_SQL, &[&name]).await
}

pub(super) async fn index_columns(client: &PgClient, name: &str) -> Vec<String> {
    text_column(client, COLUMNS_SQL, &[&name]).await
}

pub(super) async fn replica_identity(client: &PgClient, table: &str) -> String {
    first_value(client, REPLICA_SQL, &[&table]).await
}

async fn first_value<T>(client: &PgClient, statement: &str, params: &[&(dyn ToSql + Sync)]) -> T
where
    T: FromSqlOwned,
{
    client.query_one(statement, params).await.unwrap().get(0)
}

async fn optional_value<T>(
    client: &PgClient,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Option<T>
where
    T: FromSqlOwned,
{
    client
        .query_opt(statement, params)
        .await
        .unwrap()
        .map(|row| row.get(0))
}

async fn text_column(
    client: &PgClient,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Vec<String> {
    client
        .query(statement, params)
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct LogCapture(Arc<Mutex<Vec<u8>>>);

impl LogCapture {
    pub(super) fn contents(&self) -> String {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes).unwrap()
    }
}

impl Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for LogCapture {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(super) fn capture_logs() -> (LogCapture, impl Subscriber + Send + Sync) {
    let logs = LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    (logs, subscriber)
}

pub(super) fn install_logs() -> (LogCapture, DefaultGuard) {
    let (logs, subscriber) = capture_logs();
    let guard = set_default(subscriber);
    (logs, guard)
}

pub(super) fn warned(logs: &str, message: &str) -> bool {
    logs.lines()
        .any(|line| line.contains("WARN") && line.contains(message))
}

pub(super) fn informed(logs: &str, message: &str) -> bool {
    logs.lines()
        .any(|line| line.contains("INFO") && line.contains(message))
}

pub(super) fn errored(logs: &str, message: &str) -> bool {
    logs.lines()
        .any(|line| line.contains("ERROR") && line.contains(message))
}

pub(super) fn occurrences(logs: &str, needle: &str) -> usize {
    logs.lines().filter(|line| line.contains(needle)).count()
}

pub(super) async fn run_ensure(
    database: &FakeDatabase,
    indexes: &[RequestLogIndex<'_>],
    jitter: &FixedJitter,
    pause: &RecordingPause,
) -> Result<bool, FakeError> {
    let dependencies = IndexDependencies { jitter, pause };
    ensure_indexes(
        ready(Ok(database.clone())),
        "public",
        indexes,
        &dependencies,
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FakeFault {
    Database,
    LockNotAvailable,
    NotDatabase,
    Pending,
}

#[derive(Debug, thiserror::Error)]
#[error("fake database failure: {fault:?}")]
pub(super) struct FakeError {
    pub(super) fault: FakeFault,
}

impl DatabaseFailure for FakeError {
    fn is_database_failure(&self) -> bool {
        !matches!(self.fault, FakeFault::NotDatabase)
    }

    fn is_lock_not_available(&self) -> bool {
        self.fault == FakeFault::LockNotAvailable
    }
}

#[derive(Default)]
pub(super) struct FakeState {
    pub(super) tables: BTreeMap<String, Result<Option<TableKind>, FakeFault>>,
    pub(super) table_kind_sequence: VecDeque<Result<Option<TableKind>, FakeFault>>,
    pub(super) index_states: BTreeMap<String, Result<Option<IndexState>, FakeFault>>,
    pub(super) index_state_sequence: VecDeque<Result<Option<IndexState>, FakeFault>>,
    pub(super) equivalents: BTreeMap<String, Vec<EquivalentIndex>>,
    pub(super) children: BTreeMap<String, Vec<Relation>>,
    pub(super) statements: Vec<String>,
    pub(super) statement_faults: Vec<(String, FakeFault)>,
    pub(super) once_faults: Vec<(String, FakeFault)>,
    pub(super) table_kinds_asked: Vec<String>,
    pub(super) index_states_asked: Vec<String>,
    pub(super) schemas_asked: Vec<String>,
    pub(super) expected_names_asked: Vec<String>,
    pub(super) equivalent_definitions_asked: Vec<String>,
    pub(super) child_indexes_asked: Vec<String>,
    pub(super) equivalents_asked: usize,
    pub(super) lock_available: bool,
    pub(super) locked: bool,
    pub(super) lock_attempts: usize,
    pub(super) release_attempts: usize,
}

#[derive(Clone, Default)]
pub(super) struct FakeDatabase {
    pub(super) state: Arc<Mutex<FakeState>>,
}

impl FakeDatabase {
    pub(super) fn state(&self) -> MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn working(&self) -> Self {
        self.state().lock_available = true;
        self.clone()
    }

    pub(super) fn configure<F>(&self, update: F)
    where
        F: FnOnce(&mut FakeState),
    {
        let mut state = self.state();
        update(&mut state);
    }

    fn next_fault(&self, statement: &str) -> Option<FakeFault> {
        let mut state = self.state();
        state.statements.push(statement.to_owned());
        if let Some(position) = state
            .once_faults
            .iter()
            .position(|(pattern, _)| statement.contains(pattern.as_str()))
        {
            return Some(state.once_faults.remove(position).1);
        }
        state
            .statement_faults
            .iter()
            .find(|(pattern, _)| statement.contains(pattern.as_str()))
            .map(|(_, fault)| *fault)
    }

    fn acquire_coordinator(&self) -> bool {
        let mut state = self.state();
        state.lock_attempts = state.lock_attempts.saturating_add(1);
        state.locked = state.lock_available;
        state.lock_available
    }

    fn release_coordinator(&self) {
        let mut state = self.state();
        state.release_attempts = state.release_attempts.saturating_add(1);
        state.locked = false;
    }

    fn kind_answer(&self, schema: &str, table: &str) -> Result<Option<TableKind>, FakeError> {
        let mut state = self.state();
        state.schemas_asked.push(schema.to_owned());
        state.table_kinds_asked.push(table.to_owned());
        let stored = state.tables.get(table).cloned();
        answer(&mut state.table_kind_sequence, stored.as_ref())
    }

    fn state_answer(&self, schema: &str, index: &str) -> Result<Option<IndexState>, FakeError> {
        let mut state = self.state();
        state.schemas_asked.push(schema.to_owned());
        state.index_states_asked.push(index.to_owned());
        let stored = state.index_states.get(index).cloned();
        answer(&mut state.index_state_sequence, stored.as_ref())
    }

    fn equivalent_answer(
        &self,
        target: IndexRef<'_>,
        definition: &RequestLogIndex<'_>,
    ) -> Vec<EquivalentIndex> {
        let mut state = self.state();
        state.schemas_asked.push(target.schema.to_owned());
        state.expected_names_asked.push(target.name.to_owned());
        state
            .equivalent_definitions_asked
            .push(definition.name.to_owned());
        let asked = state.equivalents_asked.saturating_add(1);
        state.equivalents_asked = asked;
        state
            .equivalents
            .get(target.table)
            .cloned()
            .unwrap_or_default()
    }

    fn children_answer(&self, target: IndexRef<'_>) -> Vec<Relation> {
        let mut state = self.state();
        state.schemas_asked.push(target.schema.to_owned());
        state.child_indexes_asked.push(target.name.to_owned());
        state
            .children
            .get(target.table)
            .cloned()
            .unwrap_or_default()
    }

    async fn answer_execute(&self, statement: &str) -> Result<(), FakeError> {
        match self.next_fault(statement) {
            Some(fault) => fault_answer(fault).await,
            None => Ok(()),
        }
    }
}

fn answer<T>(
    sequence: &mut VecDeque<Result<T, FakeFault>>,
    stored: Option<&Result<T, FakeFault>>,
) -> Result<T, FakeError>
where
    T: Clone + Default,
{
    if let Some(next) = sequence.pop_front() {
        return next.map_err(|fault| FakeError { fault });
    }
    match stored {
        Some(Ok(value)) => Ok(value.clone()),
        Some(Err(fault)) => Err(FakeError { fault: *fault }),
        None => Ok(T::default()),
    }
}

impl IndexDatabase for FakeDatabase {
    type Error = FakeError;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error> {
        Box::pin(async move { self.kind_answer(schema, table) })
    }

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error> {
        Box::pin(async move { self.state_answer(schema, index) })
    }

    fn equivalent_indexes<'database>(
        &'database self,
        target: IndexRef<'database>,
        definition: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        Box::pin(async move { Ok(self.equivalent_answer(target, &definition)) })
    }

    fn children_without_the_index<'database>(
        &'database self,
        target: IndexRef<'database>,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        Box::pin(async move { Ok(self.children_answer(target)) })
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        Box::pin(self.answer_execute(statement))
    }

    fn coordinate<'database>(
        &'database self,
        step: DatabaseFuture<'database, bool, Self::Error>,
    ) -> DatabaseFuture<'database, bool, Self::Error> {
        Box::pin(async move {
            if !self.acquire_coordinator() {
                tracing::info!(
                    "Another process holds the migration lock, leaving the request-log indexes to the next index build"
                );
                return Ok(false);
            }
            let outcome = step.await;
            self.release_coordinator();
            outcome
        })
    }
}

async fn fault_answer<D>(fault: FakeFault) -> Result<D, FakeError> {
    match fault {
        FakeFault::Pending => pending::<Result<D, FakeError>>().await,
        FakeFault::Database | FakeFault::LockNotAvailable | FakeFault::NotDatabase => {
            Err(FakeError { fault })
        }
    }
}

pub(super) struct FixedJitter {
    pub(super) value: u64,
    pub(super) draws: Mutex<Vec<u64>>,
}

impl FixedJitter {
    pub(super) fn new(value: u64) -> Self {
        Self {
            value,
            draws: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn draws(&self) -> Vec<u64> {
        self.draws
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.draws
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(upper_exclusive_nanos);
        self.value.min(upper_exclusive_nanos.saturating_sub(1))
    }
}

pub(super) struct RecordingPause {
    pub(super) delays: Mutex<Vec<(Duration, bool)>>,
    pub(super) fake: FakeDatabase,
}

impl RecordingPause {
    pub(super) fn new(fake: &FakeDatabase) -> Self {
        Self {
            delays: Mutex::new(Vec::new()),
            fake: fake.clone(),
        }
    }

    pub(super) fn standalone() -> Self {
        Self::new(&FakeDatabase::default())
    }

    pub(super) fn delays(&self) -> Vec<Duration> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(delay, _)| *delay)
            .collect()
    }

    pub(super) fn locked_during(&self) -> Vec<bool> {
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, locked)| *locked)
            .collect()
    }
}

impl RetryPause for RecordingPause {
    fn pause(&self, delay: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let locked = self.fake.state().locked;
        self.delays
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((delay, locked));
        Box::pin(ready(()))
    }
}

pub(super) fn index_builder<'build, D>(
    database: &'build D,
    schema: &'build str,
    dependencies: &'build IndexDependencies<'build>,
) -> IndexBuilder<'build, D>
where
    D: IndexDatabase,
{
    IndexBuilder {
        database,
        schema,
        dependencies,
    }
}

pub(super) fn index_dependencies<'dependencies>(
    jitter: &'dependencies FixedJitter,
    pause: &'dependencies RecordingPause,
) -> IndexDependencies<'dependencies> {
    IndexDependencies { jitter, pause }
}

pub(super) fn suffix() -> &'static str {
    "api_key_startTime_idx"
}

pub(super) fn other_schema(schema: &str) -> String {
    format!("{schema}x")
}

pub(super) fn role_name(prefix: &str) -> String {
    format!("b718_{prefix}_{}", Uuid::new_v4().simple())
}

pub(super) async fn attached_children(client: &PgClient, schema: &str, index: &str) -> Vec<String> {
    text_column(client, CHILDREN_SQL, &[&index, &schema]).await
}

pub(super) async fn create_partition_tree(test: &PostgresTest) {
    let other = other_schema(&test.schema);
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"{SPEND_LOGS}\" (api_key text, \"startTime\" timestamptz, litellm_call_id text) PARTITION BY RANGE (\"startTime\");
             CREATE TABLE \"{SPEND_LOGS}_p1\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-01-01') TO ('2026-02-01') PARTITION BY RANGE (\"startTime\");
             CREATE TABLE \"{SPEND_LOGS}_p1_a\" PARTITION OF \"{SPEND_LOGS}_p1\" FOR VALUES FROM ('2026-01-01') TO ('2026-01-15');
             CREATE TABLE \"{SPEND_LOGS}_p1_b\" PARTITION OF \"{SPEND_LOGS}_p1\" FOR VALUES FROM ('2026-01-15') TO ('2026-02-01');
             CREATE TABLE \"{SPEND_LOGS}_p2\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
             CREATE SCHEMA \"{other}\";
             CREATE TABLE \"{other}\".\"{SPEND_LOGS}_x\" PARTITION OF \"{SPEND_LOGS}\" FOR VALUES FROM ('2026-03-01') TO ('2026-04-01') PARTITION BY RANGE (\"startTime\");
             CREATE TABLE \"{other}\".\"{SPEND_LOGS}_x_a\" PARTITION OF \"{other}\".\"{SPEND_LOGS}_x\" FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');"
        ))
        .await
        .unwrap();
}

pub(super) async fn oid_in_schema(client: &PgClient, schema: &str, name: &str) -> Option<u32> {
    optional_value(client, OID_IN_SCHEMA_SQL, &[&schema, &name]).await
}

pub(super) async fn run_leaf<D>(
    builder: &IndexBuilder<'_, D>,
    table: &str,
    name: &str,
    index: RequestLogIndex<'_>,
) -> bool
where
    D: IndexDatabase,
{
    let target = IndexRef {
        schema: builder.schema,
        table,
        name,
    };
    build_leaf(builder, target, index).await.unwrap()
}

pub(super) async fn run_partition<D>(builder: &IndexBuilder<'_, D>, schema: &str) -> bool
where
    D: IndexDatabase,
{
    let index = index_by_name(API_KEY_INDEX);
    build_on_partitioned_table(builder, index, partition_target(schema))
        .await
        .unwrap()
}

// A TCP listener that accepts connections and never answers the Postgres startup packet
pub(super) async fn stalled_database_url() -> (SecretString, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut sockets = JoinSet::new();
    sockets.spawn(async move {
        let mut kept = Vec::new();
        loop {
            match listener.accept().await {
                Ok((socket, _)) => keep_socket(&mut kept, socket),
                Err(_) => return,
            }
        }
    });
    let url = SecretString::from(format!(
        "postgresql://stalled:stalled@{address}/betterlite?sslmode=disable"
    ));
    (url, sockets)
}

fn keep_socket(kept: &mut Vec<TcpStream>, socket: TcpStream) {
    kept.push(socket);
    if kept.len() > 16 {
        kept.remove(0);
    }
}

pub(super) fn partition_target(schema: &str) -> PartitionTarget<'_> {
    PartitionTarget {
        schema,
        table: None,
        name: None,
    }
}

pub(super) fn plain_table() -> TableKind {
    TableKind { partitioned: false }
}

pub(super) fn partitioned_table() -> TableKind {
    TableKind { partitioned: true }
}

pub(super) fn valid_index(table: &str) -> IndexState {
    IndexState {
        valid: true,
        table: table.to_owned(),
    }
}

pub(super) fn invalid_index(table: &str) -> IndexState {
    IndexState {
        valid: false,
        table: table.to_owned(),
    }
}
