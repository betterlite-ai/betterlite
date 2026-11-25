use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::model_registration::JsonObject;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::support::{caller, capture_logs, db_model, decryption, named, names};
use crate::model_info_search::postgres::PostgresSearchDatabase;
use crate::model_info_search::search::{
    ModelFilter, SearchDatabase, SearchDatabaseError, SearchOptions, SearchSources,
    apply_search_filter,
};
use crate::model_team_scope::postgres::PostgresScopeReads;

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    litellm_params JSONB NOT NULL,
    model_info JSONB,
    blocked BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP,
    created_by TEXT,
    updated_at TIMESTAMP,
    updated_by TEXT
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    teams JSONB,
    models JSONB
);
"#;

const INSERT_MODEL_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, model_info) VALUES ($1, $2, ($3::text)::jsonb, ($4::text)::jsonb)"#;
const INSERT_USER_SQL: &str =
    r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams) VALUES ($1, ($2::text)::jsonb)"#;
const PLAINTEXT: &str = r#"{"model":"openai/gpt-4o"}"#;
const BROKEN_SECRET: &str = r#"{"model":"openai/gpt-4o","api_key":"os.environ/oidc/nope/aud"}"#;
const INVALID_PARAMS: &str = "{}";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct Database {
    fixture: PostgresFixture,
    pool: Pool,
    store: PostgresStore,
    availability: Availability,
}

impl Database {
    async fn start() -> Self {
        let url = Environment::parse_from(["model-info-search-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
                .resolve()
                .await
                .expect("the database fixture");
        let manager = Manager::new(
            fixture
                .options
                .url
                .expose_secret()
                .parse()
                .expect("the database url"),
            NoTls,
        );
        let pool = Pool::builder(manager)
            .max_size(2)
            .build()
            .expect("the database pool");
        Self {
            fixture,
            pool: pool.clone(),
            store: PostgresStore { pool: Some(pool) },
            availability: Availability::default(),
        }
    }

    fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    fn search(&self) -> PostgresSearchDatabase<'_> {
        PostgresSearchDatabase {
            client: Some(self.client()),
        }
    }

    fn scope(&self) -> PostgresScopeReads<'_> {
        PostgresScopeReads {
            client: Some(self.client()),
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn seed_model(&self, model_id: &str, model_name: &str, params: &str, info: &str) {
        self.execute(INSERT_MODEL_SQL, &[&model_id, &model_name, &params, &info])
            .await;
    }

    async fn seed_user(&self, user_id: &str, teams: &str) {
        self.execute(INSERT_USER_SQL, &[&user_id, &teams]).await;
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedCall {
    Count(ModelFilter),
    Find(ModelFilter, i64),
}

struct RecordingDatabase<'a> {
    inner: PostgresSearchDatabase<'a>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl<'a> RecordingDatabase<'a> {
    fn new(inner: PostgresSearchDatabase<'a>) -> Self {
        Self {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: RecordedCall) {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push(call);
    }

    fn calls(&self) -> Vec<RecordedCall> {
        let calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.clone()
    }
}

#[async_trait]
impl SearchDatabase for RecordingDatabase<'_> {
    async fn count(&self, filter: &ModelFilter) -> Result<i64, SearchDatabaseError> {
        self.record(RecordedCall::Count(filter.clone()));
        self.inner.count(filter).await
    }

    async fn find(
        &self,
        filter: &ModelFilter,
        take: i64,
    ) -> Result<Vec<ProxyModelTable>, SearchDatabaseError> {
        self.record(RecordedCall::Find(filter.clone(), take));
        self.inner.find(filter, take).await
    }
}

fn searching(page: i64, size: i64) -> SearchOptions<'static> {
    SearchOptions {
        page,
        size,
        sort_by: None,
        model_name: None,
    }
}

fn sorting(sort_by: &str) -> SearchOptions<'_> {
    SearchOptions {
        page: 1,
        size: 50,
        sort_by: Some(sort_by),
        model_name: None,
    }
}

fn narrowed(model_name: &str) -> SearchOptions<'_> {
    SearchOptions {
        page: 1,
        size: 50,
        sort_by: None,
        model_name: Some(model_name),
    }
}

fn sorted_names(models: &[JsonObject]) -> Vec<String> {
    let mut found = names(models);
    found.sort();
    found
}

#[tokio::test]
async fn count_before_fetch() {
    let database = Database::start().await;
    database
        .seed_model("d1", "gpt-4o-db", PLAINTEXT, "{}")
        .await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-4o")];
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &searching(1, 10)).await;
    assert_eq!(names(&outcome.models), ["gpt-4o", "gpt-4o-db"]);
    assert_eq!(outcome.total_count, Some(2));
    let calls = recorder.calls();
    let [
        RecordedCall::Count(count_filter),
        RecordedCall::Find(find_filter, take),
    ] = calls.as_slice()
    else {
        panic!("count runs before the bounded fetch");
    };
    assert_eq!(count_filter, find_filter);
    assert_eq!(*take, 9);
    assert_eq!(count_filter.search_lower, "gpt");
    assert_eq!(count_filter.exact_name, None);
    assert!(count_filter.excluded_ids.is_empty());
    database.close().await;
}

#[tokio::test]
async fn unsorted_take_zero() {
    let database = Database::start().await;
    database.seed_model("d1", "gpt-db", PLAINTEXT, "{}").await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-a"), named("gpt-b"), named("gpt-c")];
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &searching(1, 2)).await;
    assert_eq!(names(&outcome.models), ["gpt-a", "gpt-b", "gpt-c"]);
    assert_eq!(outcome.total_count, Some(4));
    let calls = recorder.calls();
    assert!(matches!(calls.as_slice(), [RecordedCall::Count(_)]));
    database.close().await;
}

#[tokio::test]
async fn sorted_cap_500() {
    let database = Database::start().await;
    database
        .seed_model("d1", "gpt-db-one", PLAINTEXT, "{}")
        .await;
    database
        .seed_model("d2", "gpt-db-two", PLAINTEXT, "{}")
        .await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-config")];
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &sorting("model_name")).await;
    assert_eq!(
        sorted_names(&outcome.models),
        ["gpt-config", "gpt-db-one", "gpt-db-two"]
    );
    assert_eq!(outcome.total_count, Some(3));
    let calls = recorder.calls();
    let [_, RecordedCall::Find(filter, take)] = calls.as_slice() else {
        panic!("a sorted search still counts then fetches");
    };
    assert_eq!(*take, 500);
    assert_eq!(filter.search_lower, "gpt");
    database.close().await;
}

#[tokio::test]
async fn unknown_sort_still_capped() {
    let database = Database::start().await;
    database.seed_model("d1", "gpt-db", PLAINTEXT, "{}").await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-config")];
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &sorting("bogus")).await;
    assert_eq!(sorted_names(&outcome.models), ["gpt-config", "gpt-db"]);
    assert_eq!(outcome.total_count, Some(2));
    let calls = recorder.calls();
    let [_, RecordedCall::Find(_, take)] = calls.as_slice() else {
        panic!("an unknown sort still fetches at the cap");
    };
    assert_eq!(*take, 500);
    database.close().await;
}

#[tokio::test]
async fn scope_before_decrypt() {
    let database = Database::start().await;
    database
        .seed_model("d1", "gpt-allowed", PLAINTEXT, r#"{"team_id":"team-a"}"#)
        .await;
    database
        .seed_model(
            "d2",
            "gpt-foreign",
            BROKEN_SECRET,
            r#"{"team_id":"team-b"}"#,
        )
        .await;
    let (captured, _guard) = capture_logs();
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let scope = database.scope();
    let who = caller("alice", "team-a");
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: Some(&scope),
        caller: Some(&who),
    };
    let outcome = apply_search_filter(&[], "gpt", Some(&sources), &searching(1, 10)).await;
    assert_eq!(names(&outcome.models), ["gpt-allowed"]);
    assert_eq!(outcome.total_count, Some(2));
    assert!(
        !captured
            .contents()
            .contains("Error querying database models with search")
    );
    database.close().await;
}

#[tokio::test]
async fn db_count_unscoped() {
    let database = Database::start().await;
    database.seed_model("d1", "gpt-db", PLAINTEXT, "{}").await;
    database
        .seed_model("d2", "gpt-skipped", INVALID_PARAMS, "{}")
        .await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let outcome = apply_search_filter(&[], "gpt", Some(&sources), &searching(1, 10)).await;
    assert_eq!(names(&outcome.models), ["gpt-db"]);
    assert_eq!(outcome.total_count, Some(2));
    database.close().await;
}

#[tokio::test]
async fn decrypt_error_discards_batch() {
    let database = Database::start().await;
    database.seed_model("d1", "gpt-good", PLAINTEXT, "{}").await;
    database
        .seed_model("d2", "gpt-broken", BROKEN_SECRET, "{}")
        .await;
    let (captured, _guard) = capture_logs();
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-config")];
    let outcome = apply_search_filter(&models, "gpt", Some(&sources), &searching(1, 10)).await;
    assert_eq!(names(&outcome.models), ["gpt-config"]);
    assert_eq!(outcome.total_count, Some(1));
    assert!(
        captured
            .contents()
            .contains("Error querying database models with search")
    );
    database.close().await;
}

#[tokio::test]
async fn exact_name_skips_db() {
    let database = Database::start().await;
    database.seed_model("d1", "gpt-4o", PLAINTEXT, "{}").await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-config")];
    let skipped =
        apply_search_filter(&models, "gpt", Some(&sources), &narrowed("other-model")).await;
    assert_eq!(names(&skipped.models), ["gpt-config"]);
    assert_eq!(skipped.total_count, Some(1));
    assert!(recorder.calls().is_empty());
    let queried = apply_search_filter(&models, "gpt", Some(&sources), &narrowed("GPT-4O")).await;
    assert_eq!(names(&queried.models), ["gpt-config"]);
    assert_eq!(queried.total_count, Some(1));
    let calls = recorder.calls();
    let [
        RecordedCall::Count(filter),
        RecordedCall::Find(fetched, take),
    ] = calls.as_slice()
    else {
        panic!("an exact name that can match still counts then fetches");
    };
    assert_eq!(filter.exact_name, Some(String::from("GPT-4O")));
    assert_eq!(fetched, filter);
    assert_eq!(*take, 49);
    database.close().await;
}

#[tokio::test]
async fn search_db_readthrough() {
    let database = Database::start().await;
    database
        .seed_model("d1", "gpt-4o-in-router", PLAINTEXT, "{}")
        .await;
    database
        .seed_model("d3", "gpt-4o-other", PLAINTEXT, "{}")
        .await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: None,
        caller: None,
    };
    let models = vec![named("gpt-4o-config"), db_model("gpt-4o-in-router", "d1")];
    let outcome = apply_search_filter(&models, "GPT-4o", Some(&sources), &searching(1, 10)).await;
    assert_eq!(
        sorted_names(&outcome.models),
        ["gpt-4o-config", "gpt-4o-in-router", "gpt-4o-other"]
    );
    assert_eq!(outcome.total_count, Some(3));
    let calls = recorder.calls();
    let [RecordedCall::Count(filter), _] = calls.as_slice() else {
        panic!("a read-through counts before fetching");
    };
    assert_eq!(filter.search_lower, "gpt-4o");
    assert_eq!(
        filter.excluded_ids.iter().cloned().collect::<Vec<String>>(),
        ["d1"]
    );
    database.close().await;
}

#[tokio::test]
async fn byok_scope_reads_user_teams() {
    let database = Database::start().await;
    database.seed_user("alice", r#"["team-b"]"#).await;
    database
        .seed_model("d1", "team-a-db", PLAINTEXT, r#"{"team_id":"team-a"}"#)
        .await;
    database
        .seed_model("d2", "team-b-db", PLAINTEXT, r#"{"team_id":"team-b"}"#)
        .await;
    database
        .seed_model("d3", "team-c-db", PLAINTEXT, r#"{"team_id":"team-c"}"#)
        .await;
    let decrypt = decryption();
    let recorder = RecordingDatabase::new(database.search());
    let scope = database.scope();
    let who = caller("alice", "team-a");
    let sources: SearchSources<'_, RecordingDatabase<'_>, PostgresScopeReads<'_>> = SearchSources {
        database: &recorder,
        decrypt: &decrypt,
        reads: Some(&scope),
        caller: Some(&who),
    };
    let outcome = apply_search_filter(&[], "team", Some(&sources), &searching(1, 10)).await;
    assert_eq!(sorted_names(&outcome.models), ["team-a-db", "team-b-db"]);
    assert_eq!(outcome.total_count, Some(3));
    database.close().await;
}
