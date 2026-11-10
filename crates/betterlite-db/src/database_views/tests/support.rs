use crate::database_views::contracts::{
    ExecuteFuture, QueryFuture, RawRow, SupportsExecuteRaw, SupportsRawQueries,
};
use crate::database_views::definitions::OFFLINE_VIEWS;
type FakeExecute<'database> = ExecuteFuture<'database, FakeFailure>;
type FakeQuery<'database> = QueryFuture<'database, FakeFailure>;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use secrecy::{ExposeSecret as _, SecretString};
use std::collections::VecDeque;
use std::future::Future;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::task::JoinSet;
use tokio_postgres::{Client, NoTls};
use tracing::instrument::WithSubscriber as _;
pub(super) const BASE_SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_VerificationToken" (
    token TEXT PRIMARY KEY,
    key_alias TEXT,
    key_name TEXT,
    team_id TEXT,
    project_id TEXT
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    spend DOUBLE PRECISION DEFAULT 0.0,
    max_budget DOUBLE PRECISION,
    model_max_budget JSONB DEFAULT '{}',
    tpm_limit BIGINT,
    rpm_limit BIGINT,
    tpd_limit BIGINT
);
CREATE TABLE "LiteLLM_ProjectTable" (
    project_id TEXT PRIMARY KEY,
    project_alias TEXT
);
CREATE TABLE "LiteLLM_SpendLogs" (
    request_id TEXT PRIMARY KEY,
    api_key TEXT NOT NULL DEFAULT '',
    spend DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    "startTime" TIMESTAMP(3) NOT NULL,
    model TEXT NOT NULL DEFAULT '',
    "user" TEXT DEFAULT '',
    request_tags JSONB DEFAULT '[]',
    end_user TEXT
);
"#;
#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}
fn postgres_test_url() -> Option<SecretString> {
    Environment::parse_from(["database-views-tests"])
        .url
        .map(SecretString::from)
}
pub(super) struct Database {
    pub(super) fixture: PostgresFixture,
    pub(super) client: Client,
    pub(super) tasks: JoinSet<Result<(), tokio_postgres::Error>>,
}
impl Database {
    pub(super) async fn new() -> Self {
        let fixture = PostgresFixture::provision_schema(postgres_test_url().as_ref())
            .resolve()
            .await
            .unwrap();
        let (client, tasks) = connect(&fixture).await;
        client.batch_execute(BASE_SCHEMA).await.unwrap();
        Self {
            fixture,
            client,
            tasks,
        }
    }
    pub(super) async fn another_client(
        &self,
    ) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
        connect(&self.fixture).await
    }
    pub(super) async fn close(self) {
        let Self {
            fixture,
            client,
            tasks,
        } = self;
        drop(client);
        drop(tasks);
        fixture.close_async().await.unwrap();
    }
}
// the fixture's Config carries a schema-mode options value the server rejects; its URL does not
async fn connect(
    fixture: &PostgresFixture,
) -> (Client, JoinSet<Result<(), tokio_postgres::Error>>) {
    let (client, connection) = tokio_postgres::connect(fixture.options.url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    tasks.spawn(connection);
    (client, tasks)
}
pub(super) async fn view_columns(client: &Client, view: &str) -> Vec<(String, String)> {
    client
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
            &[&view],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get("column_name"), row.get("data_type")))
        .collect()
}
pub(super) async fn assert_view_columns(client: &Client, view: &str, expected: &str) {
    let wanted: Vec<(String, String)> = expected
        .split(',')
        .map(|column| {
            let (name, data_type) = column.split_once(':').expect("a name:type column spec");
            (name.to_owned(), data_type.to_owned())
        })
        .collect();
    assert_eq!(view_columns(client, view).await, wanted);
}
pub(super) async fn execute(client: &Client, sql: &str) {
    client.batch_execute(sql).await.unwrap();
}
pub(super) async fn created_view_names(client: &Client) -> Vec<String> {
    let mut names: Vec<String> = client
        .query(
            "SELECT relname::text FROM pg_class \
             WHERE relnamespace = current_schema()::regnamespace AND relkind = 'v'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get("relname"))
        .collect();
    names.sort_unstable();
    names
}
pub(super) async fn create_offline_views(database: &Database) {
    for view in OFFLINE_VIEWS {
        database.client.batch_execute(view.ddl).await.unwrap();
    }
}
#[derive(Clone, Default)]
pub(super) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);
impl CapturedLogs {
    fn record(&self, bytes: &[u8]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(bytes);
    }
}
impl Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.record(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(super) async fn capture_logs<F>(future: F) -> (F::Output, Vec<String>)
where
    F: Future,
{
    let logs = CapturedLogs::default();
    let writer = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_level(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let output = future.with_subscriber(subscriber).await;
    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    (output, text.lines().map(str::to_owned).collect())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Call {
    Probe(String),
    Create(String),
}
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct FakeFailure {
    pub(super) message: String,
}
impl FakeFailure {
    pub(super) fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}
#[derive(Default)]
pub(super) struct ScriptedDatabase {
    probes: Mutex<VecDeque<Result<Vec<RawRow>, FakeFailure>>>,
    creates: Mutex<VecDeque<Result<(), FakeFailure>>>,
    calls: Mutex<Vec<Call>>,
}
impl ScriptedDatabase {
    pub(super) fn script_probe(&self, reply: Result<Vec<RawRow>, FakeFailure>) {
        self.probes.lock().unwrap().push_back(reply);
    }
    pub(super) fn script_probes_missing(&self, count: usize) {
        for _ in 0..count {
            self.script_probe(Err(FakeFailure::new("relation \"view\" does not exist")));
        }
    }
    pub(super) fn script_probe_exists(&self) {
        self.script_probe(Ok(Vec::new()));
    }
    pub(super) fn script_probes_exist(&self, count: usize) {
        for _ in 0..count {
            self.script_probe_exists();
        }
    }
    pub(super) fn script_probes_failing(&self, count: usize, message: &str) {
        for _ in 0..count {
            self.script_probe(Err(FakeFailure::new(message)));
        }
    }
    pub(super) fn script_create(&self, reply: Result<(), FakeFailure>) {
        self.creates.lock().unwrap().push_back(reply);
    }
    pub(super) fn script_creates_succeed(&self, count: usize) {
        for _ in 0..count {
            self.script_create(Ok(()));
        }
    }
    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    pub(super) fn created_ddl(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Probe(_) => None,
                Call::Create(ddl) => Some(ddl),
            })
            .collect()
    }
    fn next_create(&self) -> Result<(), FakeFailure> {
        self.creates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(FakeFailure::new("no create was scripted")))
    }
    fn next_probe(&self) -> Result<Vec<RawRow>, FakeFailure> {
        self.probes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Err(FakeFailure::new("no probe was scripted")))
    }
    fn scripted_execute<'database>(
        &'database self,
        query: &'database str,
    ) -> FakeExecute<'database> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Create(query.to_owned()));
        Box::pin(async move { self.next_create() })
    }
    fn scripted_query<'database>(&'database self, query: &'database str) -> FakeQuery<'database> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Probe(query.to_owned()));
        Box::pin(async move { self.next_probe() })
    }
}
impl SupportsExecuteRaw for ScriptedDatabase {
    type Error = FakeFailure;
    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> ExecuteFuture<'database, Self::Error> {
        self.scripted_execute(query)
    }
}
impl SupportsRawQueries for ScriptedDatabase {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
    ) -> QueryFuture<'database, Self::Error> {
        self.scripted_query(query)
    }
}
