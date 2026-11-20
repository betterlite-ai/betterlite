use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use crate::access_group_models::ModelRename;
use crate::access_group_models::postgres::PostgresModelGrants;
use crate::model_allowlist_rename::fixtures::{
    AllowlistEvent, EventLog, ObservedAllowlists, ScriptedAllowlistCache,
};
use crate::model_allowlist_rename::postgres::PostgresAllowlists;
use crate::model_allowlist_rename::{ModelAllowlistError, sync_model_allowlists_for_renamed_model};
use crate::model_visibility::testing::{Row as SnapshotRow, Snapshot};

const TEAM_TABLE: &str = "LiteLLM_TeamTable";
const KEY_TABLE: &str = "LiteLLM_VerificationToken";
const ORG_TABLE: &str = "LiteLLM_OrganizationTable";
const PROJECT_TABLE: &str = "LiteLLM_ProjectTable";
const USER_TABLE: &str = "LiteLLM_UserTable";

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_TeamTable" (
    "team_id" TEXT PRIMARY KEY,
    "team_alias" TEXT,
    "models" TEXT[]
);
CREATE TABLE "LiteLLM_VerificationToken" (
    "token" TEXT PRIMARY KEY,
    "models" TEXT[]
);
CREATE TABLE "LiteLLM_OrganizationTable" (
    "organization_id" TEXT PRIMARY KEY,
    "models" TEXT[]
);
CREATE TABLE "LiteLLM_ProjectTable" (
    "project_id" TEXT PRIMARY KEY,
    "models" TEXT[]
);
CREATE TABLE "LiteLLM_UserTable" (
    "user_id" TEXT PRIMARY KEY,
    "models" TEXT[]
);
CREATE TABLE "LiteLLM_ProxyModelTable" (
    "model_id" TEXT PRIMARY KEY,
    "model_name" TEXT NOT NULL
);
"#;
const INSERT_MODEL_SQL: &str =
    r#"INSERT INTO "LiteLLM_ProxyModelTable" ("model_id", "model_name") VALUES ($1, $2)"#;
const REFUSE_ONE_FAILURE_SQL: &str = r#"ALTER TABLE "LiteLLM_VerificationToken" ADD CONSTRAINT "b837_refuses_onefail_new" CHECK (NOT ('onefail-new' = ANY("models")))"#;
const ALLOW_ONE_FAILURE_SQL: &str =
    r#"ALTER TABLE "LiteLLM_VerificationToken" DROP CONSTRAINT "b837_refuses_onefail_new""#;
const DROP_TEAM_PK_SQL: &str =
    r#"ALTER TABLE "LiteLLM_TeamTable" DROP CONSTRAINT "LiteLLM_TeamTable_pkey""#;
const DROP_TEAM_ID_NOT_NULL_SQL: &str =
    r#"ALTER TABLE "LiteLLM_TeamTable" ALTER COLUMN "team_id" DROP NOT NULL"#;
const UNNAMED_TEAM_SQL: &str =
    r#"SELECT "models" FROM "LiteLLM_TeamTable" WHERE "team_id" IS NULL"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(super) async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["model-allowlist-rename-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
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
        .max_size(4)
        .build()
        .expect("the database pool");
    (fixture, pool)
}

pub(super) async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

async fn execute(pool: &Pool, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
    let client = pool.get().await.expect("a fixture connection");
    client
        .execute(statement, parameters)
        .await
        .expect("the fixture statement");
}

async fn insert_model(pool: &Pool, model_id: &str, model_name: &str) {
    let parameters: [&(dyn ToSql + Sync); 2] = [&model_id, &model_name];
    execute(pool, INSERT_MODEL_SQL, &parameters).await;
}

async fn insert_allowlist(
    pool: &Pool,
    table: &str,
    id_column: &str,
    id: &str,
    models: Option<&[String]>,
) {
    let statement = format!(r#"INSERT INTO "{table}" ("{id_column}", "models") VALUES ($1, $2)"#);
    let parameters: [&(dyn ToSql + Sync); 2] = [&id, &models];
    execute(pool, &statement, &parameters).await;
}

async fn insert_team(
    pool: &Pool,
    team_id: Option<&str>,
    team_alias: Option<&str>,
    models: Option<&[String]>,
) {
    let parameters: [&(dyn ToSql + Sync); 3] = [&team_id, &team_alias, &models];
    execute(
        pool,
        r#"INSERT INTO "LiteLLM_TeamTable" ("team_id", "team_alias", "models") VALUES ($1, $2, $3)"#,
        &parameters,
    )
    .await;
}

async fn stored(pool: &Pool, table: &str, id_column: &str, id: &str) -> Option<Vec<String>> {
    let statement = format!(r#"SELECT "models" FROM "{table}" WHERE "{id_column}" = $1"#);
    let parameters: [&(dyn ToSql + Sync); 1] = [&id];
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(&statement, &parameters)
        .await
        .expect("the allowlist row");
    row.get(0)
}

async fn stored_unnamed_team(pool: &Pool) -> Option<Vec<String>> {
    let client = pool.get().await.expect("a fixture connection");
    let row = client
        .query_one(UNNAMED_TEAM_SQL, &[])
        .await
        .expect("the unnamed team row");
    row.get(0)
}

fn expected(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn array(values: &[&str]) -> Option<Vec<String>> {
    Some(expected(values))
}

fn recorded_keys(cache: &ScriptedAllowlistCache) -> Vec<String> {
    let calls = cache.calls.lock().expect("calls");
    assert_eq!(calls.len(), 1, "the operation makes one eviction call");
    calls.first().cloned().unwrap_or_default()
}

// The statement's UNION ALL promises no row order, so the Postgres cases compare the key multiset.
fn assert_keys(cache: &ScriptedAllowlistCache, values: &[&str]) {
    let mut keys = recorded_keys(cache);
    keys.sort();
    let mut wanted = expected(values);
    wanted.sort();
    assert_eq!(keys, wanted);
}

fn catalog(rows: &[(&str, &str)]) -> DeploymentCatalog {
    let params = JsonObject::new();
    let infos: Vec<Json> = rows.iter().map(|(_, id)| info(id)).collect();
    let snapshot: Vec<SnapshotRow<'_>> = rows
        .iter()
        .zip(&infos)
        .map(|((model_name, _), model_info)| SnapshotRow {
            model_name,
            litellm_params: &params,
            model_info,
        })
        .collect();
    Snapshot::build(&snapshot, &JsonObject::new())
        .expect("the catalog snapshot")
        .catalog
}

fn info(id: &str) -> Json {
    let mut model_info = JsonObject::new();
    model_info.insert("id".to_owned(), Json::Text(id.to_owned()));
    Json::Object(model_info)
}

struct Harness {
    store: PostgresStore,
    availability: Availability,
    events: EventLog,
}

impl Harness {
    fn new(pool: &Pool) -> Self {
        Self {
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            availability: Availability::default(),
            events: EventLog::default(),
        }
    }

    fn cache(&self) -> ScriptedAllowlistCache {
        ScriptedAllowlistCache::recording(self.events.clone())
    }

    async fn run(
        &self,
        rename: &ModelRename<'_>,
        cache: &ScriptedAllowlistCache,
        catalog: Option<&DeploymentCatalog>,
    ) -> Result<(), ModelAllowlistError> {
        let client = PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        };
        let grants = PostgresModelGrants {
            client: Some(client),
        };
        let allowlists = ObservedAllowlists {
            inner: PostgresAllowlists {
                client: Some(client),
            },
            events: self.events.clone(),
        };
        sync_model_allowlists_for_renamed_model(&grants, &allowlists, cache, catalog, rename).await
    }
}

fn request<'request>(
    model_id: &'request str,
    old_name: &'request str,
    new_name: &'request str,
) -> ModelRename<'request> {
    ModelRename {
        model_id,
        old_name,
        new_name,
    }
}

pub(super) async fn pinned_rows(pool: &Pool) {
    equal_names(pool).await;
    database_sibling(pool).await;
    config_sibling(pool).await;
    last_sibling(pool).await;
    duplicate_old(pool).await;
    existing_new(pool).await;
    no_old(pool).await;
    null_array(pool).await;
    empty_array(pool).await;
    literal_wildcard(pool).await;
    quoted_name(pool).await;
    pinned_writer(pool).await;
}

async fn equal_names(pool: &Pool) {
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "same-name", "same-name");
    harness.run(&rename, &cache, None).await.unwrap();
    assert!(cache.calls.lock().expect("calls").is_empty());
    assert!(
        harness.events.recorded().is_empty(),
        "equal names return before the sibling query, the statement and the cache"
    );
}

async fn database_sibling(pool: &Pool) {
    insert_model(pool, "dep-sib", "sib-new").await;
    insert_model(pool, "dep-sib-other", "sib-old").await;
    insert_team(
        pool,
        Some("t-sib"),
        None,
        array(&["sib-old", "x"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-sib",
        array(&["sib-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        ORG_TABLE,
        "organization_id",
        "o-sib",
        array(&["sib-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        PROJECT_TABLE,
        "project_id",
        "p-sib",
        array(&["sib-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        USER_TABLE,
        "user_id",
        "u-sib",
        array(&["sib-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-sib", "sib-old", "sib-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, TEAM_TABLE, "team_id", "t-sib").await,
        array(&["sib-old", "x", "sib-new"])
    );
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-sib").await,
        array(&["sib-old", "sib-new"])
    );
    assert_eq!(
        stored(pool, ORG_TABLE, "organization_id", "o-sib").await,
        array(&["sib-old", "sib-new"])
    );
    assert_eq!(
        stored(pool, PROJECT_TABLE, "project_id", "p-sib").await,
        array(&["sib-old", "sib-new"])
    );
    assert_eq!(
        stored(pool, USER_TABLE, "user_id", "u-sib").await,
        array(&["sib-old", "sib-new"])
    );
    assert_keys(
        &cache,
        &[
            "team_id:t-sib",
            "k-sib",
            "org_id:o-sib",
            "org_id:o-sib:with_budget",
            "project_id:p-sib",
            "u-sib",
        ],
    );
}

async fn config_sibling(pool: &Pool) {
    insert_team(
        pool,
        Some("t-cfg"),
        Some("cfg-alias"),
        array(&["cfg-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        USER_TABLE,
        "user_id",
        "u-cfg",
        array(&["cfg-old"]).as_deref(),
    )
    .await;
    let router_catalog = catalog(&[("cfg-old", "dep-cfg")]);
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "cfg-old", "cfg-new");
    harness
        .run(&rename, &cache, Some(&router_catalog))
        .await
        .unwrap();
    assert_eq!(
        stored(pool, TEAM_TABLE, "team_id", "t-cfg").await,
        array(&["cfg-old", "cfg-new"])
    );
    assert_eq!(
        stored(pool, USER_TABLE, "user_id", "u-cfg").await,
        array(&["cfg-old", "cfg-new"])
    );
    assert_keys(&cache, &["team_id:t-cfg", "team_alias:cfg-alias", "u-cfg"]);
}

async fn last_sibling(pool: &Pool) {
    insert_model(pool, "dep-last", "last-new").await;
    insert_allowlist(
        pool,
        PROJECT_TABLE,
        "project_id",
        "p-last",
        array(&["last-old", "last-old", "z"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        ORG_TABLE,
        "organization_id",
        "o-last",
        array(&["last-new", "last-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-last", "last-old", "last-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, PROJECT_TABLE, "project_id", "p-last").await,
        array(&["last-new", "last-new", "z"])
    );
    assert_eq!(
        stored(pool, ORG_TABLE, "organization_id", "o-last").await,
        array(&["last-new"])
    );
    assert_keys(
        &cache,
        &[
            "project_id:p-last",
            "org_id:o-last",
            "org_id:o-last:with_budget",
        ],
    );
}

async fn duplicate_old(pool: &Pool) {
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-dup",
        array(&["dup-old", "x", "dup-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "dup-old", "dup-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-dup").await,
        array(&["dup-new", "x", "dup-new"])
    );
    assert_keys(&cache, &["k-dup"]);
}

async fn existing_new(pool: &Pool) {
    insert_model(pool, "dep-exist", "exist-new").await;
    insert_model(pool, "dep-exist-sibling", "exist-old").await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-exist-both",
        array(&["exist-new", "exist-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-exist-append",
        array(&["exist-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-exist", "exist-old", "exist-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-exist-both").await,
        array(&["exist-new", "exist-old"])
    );
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-exist-append").await,
        array(&["exist-old", "exist-new"])
    );
    assert_keys(&cache, &["k-exist-append"]);
}

async fn no_old(pool: &Pool) {
    insert_allowlist(
        pool,
        USER_TABLE,
        "user_id",
        "u-no-old",
        array(&["unrelated"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "no-old", "no-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, USER_TABLE, "user_id", "u-no-old").await,
        array(&["unrelated"])
    );
    assert_keys(&cache, &[]);
}

async fn null_array(pool: &Pool) {
    insert_team(pool, Some("t-null"), None, None).await;
    insert_allowlist(pool, KEY_TABLE, "token", "k-null", None).await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "null-old", "null-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(stored(pool, TEAM_TABLE, "team_id", "t-null").await, None);
    assert_eq!(stored(pool, KEY_TABLE, "token", "k-null").await, None);
    assert_keys(&cache, &[]);
}

async fn empty_array(pool: &Pool) {
    insert_allowlist(
        pool,
        USER_TABLE,
        "user_id",
        "u-empty",
        array(&[]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "empty-old", "empty-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, USER_TABLE, "user_id", "u-empty").await,
        array(&[])
    );
    assert_keys(&cache, &[]);
}

async fn literal_wildcard(pool: &Pool) {
    insert_allowlist(
        pool,
        PROJECT_TABLE,
        "project_id",
        "p-wild-new",
        array(&["gpt-4o"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        PROJECT_TABLE,
        "project_id",
        "p-wild-literal",
        array(&["gpt-*"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "gpt-*", "gpt-4o");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, PROJECT_TABLE, "project_id", "p-wild-new").await,
        array(&["gpt-4o"])
    );
    assert_eq!(
        stored(pool, PROJECT_TABLE, "project_id", "p-wild-literal").await,
        array(&["gpt-4o"])
    );
    assert_keys(&cache, &["project_id:p-wild-literal"]);
}

async fn quoted_name(pool: &Pool) {
    let old_name = "it's \"quoted\" $1";
    let new_name = "new 'name'";
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-quote",
        array(&[old_name]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-quote-other",
        array(&["it"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", old_name, new_name);
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-quote").await,
        array(&[new_name])
    );
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-quote-other").await,
        array(&["it"])
    );
    assert_keys(&cache, &["k-quote"]);
}

async fn pinned_writer(pool: &Pool) {
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-pinned",
        array(&["pin-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    harness
        .availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let cache = harness.cache();
    let rename = request("dep-1", "pin-old", "pin-new");
    harness.run(&rename, &cache, None).await.unwrap();
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-pinned").await,
        array(&["pin-new"])
    );
}

pub(super) async fn failure_effects(pool: &Pool) {
    one_table_failure(pool).await;
    eviction_failure(pool).await;
    bad_returned_row(pool).await;
}

async fn one_table_failure(pool: &Pool) {
    insert_team(
        pool,
        Some("t-one"),
        None,
        array(&["onefail-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-one",
        array(&["onefail-old"]).as_deref(),
    )
    .await;
    execute(pool, REFUSE_ONE_FAILURE_SQL, &[]).await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "onefail-old", "onefail-new");
    let error = harness
        .run(&rename, &cache, None)
        .await
        .expect_err("the key table refuses the rewritten row");
    execute(pool, ALLOW_ONE_FAILURE_SQL, &[]).await;
    assert!(matches!(error, ModelAllowlistError::Database(_)));
    assert_eq!(
        stored(pool, TEAM_TABLE, "team_id", "t-one").await,
        array(&["onefail-old"]),
        "one CTE's failure rolls back the team table's update too"
    );
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-one").await,
        array(&["onefail-old"])
    );
    assert!(cache.calls.lock().expect("calls").is_empty());
    assert!(harness.events.recorded().is_empty());
}

async fn eviction_failure(pool: &Pool) {
    insert_team(
        pool,
        Some("t-evict"),
        Some("evict-alias"),
        array(&["evict-old"]).as_deref(),
    )
    .await;
    insert_allowlist(
        pool,
        KEY_TABLE,
        "token",
        "k-evict",
        array(&["evict-old"]).as_deref(),
    )
    .await;
    let harness = Harness::new(pool);
    let cache = ScriptedAllowlistCache::failing(harness.events.clone(), 3);
    let rename = request("dep-1", "evict-old", "evict-new");
    let error = harness
        .run(&rename, &cache, None)
        .await
        .expect_err("the eviction reports an error");
    assert!(matches!(error, ModelAllowlistError::Cache(_)));
    assert_eq!(
        stored(pool, TEAM_TABLE, "team_id", "t-evict").await,
        array(&["evict-new"])
    );
    assert_eq!(
        stored(pool, KEY_TABLE, "token", "k-evict").await,
        array(&["evict-new"])
    );
    let mut keys = recorded_keys(&cache);
    keys.sort();
    assert_eq!(
        keys,
        expected(&["k-evict", "team_alias:evict-alias", "team_id:t-evict"])
    );
    let recorded = harness.events.recorded();
    assert_eq!(recorded.first(), Some(&AllowlistEvent::SqlCompleted));
    assert_eq!(
        recorded.len(),
        2,
        "the statement completes once, then the one cache call reports the error"
    );
}

async fn bad_returned_row(pool: &Pool) {
    execute(pool, DROP_TEAM_PK_SQL, &[]).await;
    execute(pool, DROP_TEAM_ID_NOT_NULL_SQL, &[]).await;
    insert_team(pool, None, None, array(&["badrow-old"]).as_deref()).await;
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "badrow-old", "badrow-new");
    let error = harness
        .run(&rename, &cache, None)
        .await
        .expect_err("the NULL team id cannot be decoded");
    assert!(matches!(error, ModelAllowlistError::Database(_)));
    assert_eq!(
        stored_unnamed_team(pool).await,
        array(&["badrow-new"]),
        "the statement committed before the row failed to decode"
    );
    assert!(cache.calls.lock().expect("calls").is_empty());
}
