use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, pending, ready};
use std::io::{Result as IoResult, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::NoTls;
use tracing::Subscriber;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

use crate::migration_ledger::lock::JitterSource;
use crate::request_log_index_catalog::catalog::{EquivalentIndex, IndexState, Relation, TableKind};
use crate::request_log_index_catalog::definitions::{REQUEST_LOG_INDEXES, RequestLogIndex};
use crate::request_log_indexes::connect;
use crate::request_log_indexes::database::{
    DatabaseFailure, DatabaseFuture, IndexDatabase, PostgresDatabase,
};
use crate::request_log_indexes::ensure::ensure_indexes;
use crate::request_log_indexes::leaf::build_leaf;
use crate::request_log_indexes::locks::RetryPause;
use crate::request_log_indexes::partition::build_on_partitioned_table;
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

pub(super) struct PostgresTest {
    pub(super) fixture: PostgresFixture,
    pub(super) schema: String,
    pub(super) client: tokio_postgres::Client,
    drivers: JoinSet<Result<(), tokio_postgres::Error>>,
}

impl PostgresTest {
    pub(super) async fn new() -> Self {
        let fixture = PostgresFixture::provision_schema(postgres_test_url().as_ref())
            .resolve()
            .await
            .unwrap();
        let mut drivers = JoinSet::new();
        let (client, connection) =
            tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
                .await
                .unwrap();
        drivers.spawn(connection);
        let schema: String = client
            .query_one("SELECT current_schema()", &[])
            .await
            .unwrap()
            .get(0);
        Self {
            fixture,
            schema,
            client,
            drivers,
        }
    }

    pub(super) async fn service(&self) -> PostgresDatabase {
        connect::connect(&self.fixture.options.url).await.unwrap()
    }

    pub(super) async fn another_client(
        &self,
    ) -> (
        tokio_postgres::Client,
        JoinSet<Result<(), tokio_postgres::Error>>,
    ) {
        let (client, connection) =
            tokio_postgres::connect(self.fixture.options.url.expose_secret(), NoTls)
                .await
                .unwrap();
        let mut drivers = JoinSet::new();
        drivers.spawn(connection);
        (client, drivers)
    }

    pub(super) async fn finish(self) {
        let Self {
            fixture,
            client,
            drivers,
            ..
        } = self;
        drop(client);
        drop(drivers);
        fixture.close_async().await.unwrap();
    }
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
        schema: &'database str,
        table: &'database str,
        expected_name: &'database str,
        index: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        self.inner
            .equivalent_indexes(schema, table, expected_name, index)
    }

    fn children_without_the_index<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        self.inner.children_without_the_index(schema, table, index)
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        Box::pin(async move {
            self.statements
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(statement.to_owned());
            self.inner.execute(statement).await
        })
    }

    fn try_coordinator_lock(&self) -> DatabaseFuture<'_, bool, Self::Error> {
        self.inner.try_coordinator_lock()
    }

    fn release_coordinator_lock(&self) -> DatabaseFuture<'_, (), Self::Error> {
        self.inner.release_coordinator_lock()
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

pub(super) async fn create_spend_logs(test: &PostgresTest) {
    test.client
        .batch_execute(&format!(
            "CREATE TABLE \"{SPEND_LOGS}\" (request_id text, api_key text, \"startTime\" timestamptz, litellm_call_id text);"
        ))
        .await
        .unwrap();
}

pub(super) async fn duplicate_api_key_rows(client: &tokio_postgres::Client, table: &str) {
    client
        .batch_execute(&format!(
            "INSERT INTO \"{table}\" (request_id, api_key, \"startTime\", litellm_call_id) VALUES ('one', 'same', '2026-01-01T00:00:00Z', 'call-one'), ('two', 'same', '2026-01-01T00:00:00Z', 'call-two');"
        ))
        .await
        .unwrap();
}

// A UNIQUE CONCURRENTLY build over duplicate rows fails and leaves the named index invalid
pub(super) async fn leave_invalid_index(
    client: &tokio_postgres::Client,
    name: &str,
    table: &str,
    columns: &str,
) {
    client
        .batch_execute(&format!(
            "CREATE UNIQUE INDEX CONCURRENTLY \"{name}\" ON \"{table}\" ({columns})"
        ))
        .await
        .unwrap_err();
}

pub(super) async fn index_state_of(
    client: &tokio_postgres::Client,
    name: &str,
) -> Option<(String, bool)> {
    client
        .query_opt(
            "SELECT t.relname AS \"table\", x.indisvalid AS valid FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid JOIN pg_class t ON t.oid = x.indrelid WHERE i.relname = $1 AND i.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())",
            &[&name],
        )
        .await
        .unwrap()
        .map(|row| (row.get("table"), row.get("valid")))
}

pub(super) async fn index_oid(client: &tokio_postgres::Client, name: &str) -> Option<u32> {
    client
        .query_opt(
            "SELECT c.oid FROM pg_class c WHERE c.relname = $1 AND c.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())",
            &[&name],
        )
        .await
        .unwrap()
        .map(|row| row.get(0))
}

pub(super) async fn relation_kind(client: &tokio_postgres::Client, name: &str) -> String {
    client
        .query_one(
            "SELECT relkind::text FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())",
            &[&name],
        )
        .await
        .unwrap()
        .get(0)
}

pub(super) async fn index_columns(client: &tokio_postgres::Client, name: &str) -> Vec<String> {
    let rowset = client
        .query(
            "SELECT a.attname FROM pg_index x JOIN pg_attribute a ON a.attrelid = x.indrelid AND a.attnum = ANY (x.indkey::int2[]) WHERE x.indexrelid = (SELECT oid FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema()))",
            &[&name],
        )
        .await
        .unwrap();
    rowset.iter().map(|row| row.get(0)).collect()
}

pub(super) async fn replica_identity(client: &tokio_postgres::Client, table: &str) -> String {
    client
        .query_one(
            "SELECT relreplident::text FROM pg_class WHERE relname = $1 AND relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = current_schema())",
            &[&table],
        )
        .await
        .unwrap()
        .get(0)
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
}

impl IndexDatabase for FakeDatabase {
    type Error = FakeError;

    fn table_kind<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
    ) -> DatabaseFuture<'database, Option<TableKind>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state();
            state.schemas_asked.push(schema.to_owned());
            state.table_kinds_asked.push(table.to_owned());
            if let Some(answer) = state.table_kind_sequence.pop_front() {
                return answer.map_err(|fault| FakeError { fault });
            }
            match state.tables.get(table) {
                Some(Ok(kind)) => Ok(kind.clone()),
                Some(Err(fault)) => Err(FakeError { fault: *fault }),
                None => Ok(None),
            }
        })
    }

    fn index_state<'database>(
        &'database self,
        schema: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Option<IndexState>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state();
            state.schemas_asked.push(schema.to_owned());
            state.index_states_asked.push(index.to_owned());
            if let Some(answer) = state.index_state_sequence.pop_front() {
                return answer.map_err(|fault| FakeError { fault });
            }
            match state.index_states.get(index) {
                Some(Ok(found)) => Ok(found.clone()),
                Some(Err(fault)) => Err(FakeError { fault: *fault }),
                None => Ok(None),
            }
        })
    }

    fn equivalent_indexes<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        expected_name: &'database str,
        index: RequestLogIndex<'database>,
    ) -> DatabaseFuture<'database, Vec<EquivalentIndex>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state();
            state.schemas_asked.push(schema.to_owned());
            state.expected_names_asked.push(expected_name.to_owned());
            state
                .equivalent_definitions_asked
                .push(index.name.to_owned());
            state.equivalents_asked = state.equivalents_asked.saturating_add(1);
            Ok(state.equivalents.get(table).cloned().unwrap_or_default())
        })
    }

    fn children_without_the_index<'database>(
        &'database self,
        schema: &'database str,
        table: &'database str,
        index: &'database str,
    ) -> DatabaseFuture<'database, Vec<Relation>, Self::Error> {
        Box::pin(async move {
            let mut state = self.state();
            state.schemas_asked.push(schema.to_owned());
            state.child_indexes_asked.push(index.to_owned());
            Ok(state.children.get(table).cloned().unwrap_or_default())
        })
    }

    fn execute<'database>(
        &'database self,
        statement: &'database str,
    ) -> DatabaseFuture<'database, (), Self::Error> {
        Box::pin(async move {
            match self.next_fault(statement) {
                Some(fault) => fault_answer(fault).await,
                None => Ok(()),
            }
        })
    }

    fn try_coordinator_lock(&self) -> DatabaseFuture<'_, bool, Self::Error> {
        Box::pin(async move { Ok(self.acquire_coordinator()) })
    }

    fn release_coordinator_lock(&self) -> DatabaseFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.release_coordinator();
            Ok(())
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

pub(super) async fn attached_children(
    client: &tokio_postgres::Client,
    schema: &str,
    index: &str,
) -> Vec<String> {
    let rowset = client
        .query(
            "SELECT child.relname FROM pg_inherits i JOIN pg_class parent ON parent.oid = i.inhparent JOIN pg_class child ON child.oid = i.inhrelid WHERE parent.relname = $1 AND parent.relnamespace = (SELECT oid FROM pg_namespace WHERE nspname = $2) ORDER BY child.relname",
            &[&index, &schema],
        )
        .await
        .unwrap();
    rowset.iter().map(|row| row.get(0)).collect()
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

pub(super) async fn oid_in_schema(
    client: &tokio_postgres::Client,
    schema: &str,
    name: &str,
) -> Option<u32> {
    client
        .query_opt(
            "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &name],
        )
        .await
        .unwrap()
        .map(|row| row.get(0))
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
    build_leaf(builder, table, name, index).await.unwrap()
}

pub(super) async fn run_partition<D>(
    builder: &IndexBuilder<'_, D>,
    index: RequestLogIndex<'_>,
) -> bool
where
    D: IndexDatabase,
{
    build_on_partitioned_table(builder, index, None, None)
        .await
        .unwrap()
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
