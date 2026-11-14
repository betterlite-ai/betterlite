#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use betterlite_lens::models::decode::decode;
use betterlite_lens::models::job::{Job, Lens, Worker};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::database::{Database, ExecuteFuture, QueryFuture, WriterDatabase};
use crate::lens_repository::repository::LensRepository;

const TABLES: &str = r#"
CREATE TABLE "LiteLLM_Lens" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "version" INTEGER NOT NULL DEFAULT 0,
    "data" JSONB NOT NULL
);
CREATE TABLE "LiteLLM_LensRun" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "lens_id" TEXT NOT NULL,
    "created_at" TIMESTAMP(3) NOT NULL,
    "data" JSONB NOT NULL
);
CREATE TABLE "LiteLLM_LensWorker" (
    "id" TEXT NOT NULL PRIMARY KEY,
    "token_hash" TEXT NOT NULL UNIQUE,
    "data" JSONB NOT NULL
);
"#;

const INSERT_LENS: &str =
    r#"INSERT INTO "LiteLLM_Lens" (id, version, data) VALUES ($1, $2::bigint, $3::text::jsonb)"#;

const INSERT_RUN: &str = r#"INSERT INTO "LiteLLM_LensRun" (id, lens_id, created_at, data)
VALUES ($1, $2, $3::text::timestamp, $4::text::jsonb)"#;

const INSERT_WORKER: &str =
    r#"INSERT INTO "LiteLLM_LensWorker" (id, token_hash, data) VALUES ($1, $2, $3::text::jsonb)"#;

const STORED_RUNS: &str =
    r#"SELECT data::text FROM "LiteLLM_LensRun" WHERE lens_id=$1 ORDER BY id"#;

const STORED_WORKERS: &str =
    r#"SELECT token_hash, data::text FROM "LiteLLM_LensWorker" ORDER BY id"#;

pub(crate) const LENS_ID: &str = "lens-a";

pub(crate) type TestRepository = LensRepository<WriterDatabase<Client>>;

pub(crate) type RacingRepository = LensRepository<Racer<Recorder<WriterDatabase<Client>>>>;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct TestDatabase {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
}

impl TestDatabase {
    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

pub(crate) async fn database() -> TestDatabase {
    let environment = Environment::parse_from(["lens-repository-tests"]);
    let url = environment.url.map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(4).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(TABLES).await.unwrap();
    drop(client);
    TestDatabase { fixture, pool }
}

pub(crate) async fn repository(pool: &Pool) -> TestRepository {
    let writer = pool.get().await.unwrap();
    LensRepository {
        database: WriterDatabase { writer },
    }
}

pub(crate) fn lens_json(id: &str, jobs: &str) -> String {
    format!(
        r#"{{"id":"{id}","scope":{{}},"settings":{{"name":"lens","model":"gpt-4o","context":"cite"}},"created_at":"2026-01-01T00:00:00Z","next_run_at":"2026-01-01T00:15:00Z","budget_month":"2026-01","jobs":{jobs}}}"#
    )
}

pub(crate) fn lens(id: &str) -> Lens {
    decode::<Lens>(lens_json(id, "[]").as_bytes()).unwrap()
}

pub(crate) fn lens_with_jobs(id: &str, jobs: &str) -> Lens {
    decode::<Lens>(lens_json(id, jobs).as_bytes()).unwrap()
}

pub(crate) fn job_json(id: &str, created_at: &str, revision: i64) -> String {
    format!(
        r#"{{"id":"{id}","created_at":"{created_at}","start":"{created_at}","end":"{created_at}","settings":{{"name":"lens","model":"gpt-4o","context":"cite"}},"revision":{revision}}}"#
    )
}

pub(crate) fn job(id: &str, created_at: &str) -> Job {
    decode::<Job>(job_json(id, created_at, 1).as_bytes()).unwrap()
}

pub(crate) fn worker_json(id: &str, last_seen: &str, revoked: bool) -> String {
    format!(
        r#"{{"id":"{id}","name":"worker","scope":{{}},"last_seen":"{last_seen}","revoked":{revoked}}}"#
    )
}

pub(crate) fn worker(id: &str, last_seen: &str) -> Worker {
    decode::<Worker>(worker_json(id, last_seen, false).as_bytes()).unwrap()
}

pub(crate) async fn insert_lens(pool: &Pool, lens: &Lens, version: i64) {
    let client = pool.get().await.unwrap();
    let data = serde_json::to_string(lens).unwrap();
    client
        .execute(INSERT_LENS, &[&lens.id, &version, &data])
        .await
        .unwrap();
}

pub(crate) async fn insert_run(pool: &Pool, lens_id: &str, job: &Job) {
    let client = pool.get().await.unwrap();
    insert_run_with(&client, lens_id, job).await;
}

pub(crate) async fn insert_run_with(client: &Client, lens_id: &str, job: &Job) {
    let data = serde_json::to_string(job).unwrap();
    let created_at = job.created_at.to_rfc3339();
    client
        .execute(INSERT_RUN, &[&job.id, &lens_id, &created_at, &data])
        .await
        .unwrap();
}

pub(crate) async fn insert_worker(pool: &Pool, worker: &Worker, token_hash: &str) {
    let client = pool.get().await.unwrap();
    let data = serde_json::to_string(worker).unwrap();
    client
        .execute(INSERT_WORKER, &[&worker.id, &token_hash, &data])
        .await
        .unwrap();
}

pub(crate) async fn insert_worker_raw(pool: &Pool, id: &str, token_hash: &str, data: &str) {
    let client = pool.get().await.unwrap();
    client
        .execute(
            r#"INSERT INTO "LiteLLM_LensWorker" (id, token_hash, data) VALUES ($1, $2, $3::text::jsonb)"#,
            &[&id, &token_hash, &data],
        )
        .await
        .unwrap();
}

pub(crate) async fn stored_lens(pool: &Pool, lens_id: &str) -> (i64, Lens) {
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            r#"SELECT version::bigint, data::text FROM "LiteLLM_Lens" WHERE id=$1"#,
            &[&lens_id],
        )
        .await
        .unwrap();
    let version: i64 = row.get(0);
    let data: String = row.get(1);
    (version, decode::<Lens>(data.as_bytes()).unwrap())
}

pub(crate) async fn stored_runs(pool: &Pool, lens_id: &str) -> Vec<Job> {
    let client = pool.get().await.unwrap();
    let rows = client.query(STORED_RUNS, &[&lens_id]).await.unwrap();
    rows.iter()
        .map(|row| decode::<Job>(row.get::<_, String>(0).as_bytes()).unwrap())
        .collect()
}

pub(crate) async fn stored_workers(pool: &Pool) -> Vec<(String, Worker)> {
    let client = pool.get().await.unwrap();
    let rows = client.query(STORED_WORKERS, &[]).await.unwrap();
    rows.iter()
        .map(|row| {
            (
                row.get(0),
                decode::<Worker>(row.get::<_, String>(1).as_bytes()).unwrap(),
            )
        })
        .collect()
}

pub(crate) async fn stored_worker_text(pool: &Pool, id: &str, field: &str) -> String {
    let client = pool.get().await.unwrap();
    let query = format!(r#"SELECT data->>'{field}' FROM "LiteLLM_LensWorker" WHERE id=$1"#);
    client.query_one(&query, &[&id]).await.unwrap().get(0)
}

pub(crate) struct Recorder<Db> {
    pub(crate) database: Db,
    pub(crate) queries: Arc<Mutex<Vec<String>>>,
}

impl<Db: Database> Database for Recorder<Db> {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        self.queries.lock().unwrap().push(query.to_owned());
        self.database.query_raw(query, args)
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        self.database.execute_raw(query, args)
    }
}

pub(crate) struct Racer<Db> {
    pub(crate) database: Db,
    pub(crate) pool: Pool,
    pub(crate) lens_id: String,
    pub(crate) races: Arc<AtomicUsize>,
}

impl<Db: Database> Database for Racer<Db> {
    fn query_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> QueryFuture<'database> {
        if query.starts_with("WITH previous") && self.races.load(Ordering::SeqCst) > 0 {
            self.races.fetch_sub(1, Ordering::SeqCst);
            let pool = self.pool.clone();
            let lens_id = self.lens_id.clone();
            return Box::pin(async move {
                let client = pool.get().await.unwrap();
                // a concurrent writer bumps the column and the record's version together
                client
                    .execute(
                        r#"UPDATE "LiteLLM_Lens" SET version = version + 1,
                           data = jsonb_set(data, '{version}', to_jsonb(version + 1)) WHERE id = $1"#,
                        &[&lens_id],
                    )
                    .await
                    .unwrap();
                drop(client);
                self.database.query_raw(query, args).await
            });
        }
        self.database.query_raw(query, args)
    }

    fn execute_raw<'database>(
        &'database self,
        query: &'database str,
        args: &'database [SqlBind],
    ) -> ExecuteFuture<'database> {
        self.database.execute_raw(query, args)
    }
}

pub(crate) struct Racing {
    pub(crate) repository: RacingRepository,
    pub(crate) races: Arc<AtomicUsize>,
}

pub(crate) async fn racing(pool: &Pool, lens_id: &str, races: usize) -> Racing {
    let remaining = Arc::new(AtomicUsize::new(races));
    let recorder = Recorder {
        database: WriterDatabase {
            writer: pool.get().await.unwrap(),
        },
        queries: Arc::new(Mutex::new(Vec::new())),
    };
    let contender = Racer {
        database: recorder,
        pool: pool.clone(),
        lens_id: lens_id.to_owned(),
        races: Arc::clone(&remaining),
    };
    Racing {
        repository: LensRepository {
            database: contender,
        },
        races: remaining,
    }
}
