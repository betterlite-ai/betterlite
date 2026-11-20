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
use crate::model_allowlist_rename::{ModelAllowlistError, sync_renamed_model_allowlists};
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
        sync_renamed_model_allowlists(&grants, &allowlists, cache, catalog, rename).await
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
    run_pinned_case(pool, &DATABASE_SIBLING).await;
    run_pinned_case(pool, &CONFIG_SIBLING).await;
    run_pinned_case(pool, &LAST_SIBLING).await;
    run_pinned_case(pool, &DUPLICATE_OLD).await;
    run_pinned_case(pool, &EXISTING_NEW).await;
    run_pinned_case(pool, &NO_OLD).await;
    run_pinned_case(pool, &NULL_ARRAY).await;
    run_pinned_case(pool, &EMPTY_ARRAY).await;
    run_pinned_case(pool, &LITERAL_WILDCARD).await;
    run_pinned_case(pool, &QUOTED_NAME).await;
    pinned_writer(pool).await;
}

async fn equal_names(pool: &Pool) {
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request("dep-1", "same-name", "same-name");
    harness.run(&rename, &cache, None).await.unwrap();
    assert!(cache.calls.lock().expect("calls").is_empty());
    assert!(
        harness.events.lock().expect("events").is_empty(),
        "equal names return before the sibling query, the statement and the cache"
    );
}

type AllowlistModels = Option<&'static [&'static str]>;
type AllowlistRow = (&'static str, &'static str, &'static str, AllowlistModels);
type TeamRow = (Option<&'static str>, Option<&'static str>, AllowlistModels);

// Each case seeds its rows, renames one model, then pins the persisted arrays and the evicted keys.
struct PinnedCase {
    name: &'static str,
    deployments: &'static [(&'static str, &'static str)],
    teams: &'static [TeamRow],
    allowlists: &'static [AllowlistRow],
    catalog: &'static [(&'static str, &'static str)],
    model_id: &'static str,
    old_name: &'static str,
    new_name: &'static str,
    stored: &'static [AllowlistRow],
    keys: &'static [&'static str],
}

const DATABASE_SIBLING: PinnedCase = PinnedCase {
    name: "database_sibling",
    deployments: &[("dep-sib", "sib-new"), ("dep-sib-other", "sib-old")],
    teams: &[(Some("t-sib"), None, Some(&["sib-old", "x"]))],
    allowlists: &[
        (KEY_TABLE, "token", "k-sib", Some(&["sib-old"])),
        (ORG_TABLE, "organization_id", "o-sib", Some(&["sib-old"])),
        (PROJECT_TABLE, "project_id", "p-sib", Some(&["sib-old"])),
        (USER_TABLE, "user_id", "u-sib", Some(&["sib-old"])),
    ],
    catalog: &[],
    model_id: "dep-sib",
    old_name: "sib-old",
    new_name: "sib-new",
    stored: &[
        (
            TEAM_TABLE,
            "team_id",
            "t-sib",
            Some(&["sib-old", "x", "sib-new"]),
        ),
        (KEY_TABLE, "token", "k-sib", Some(&["sib-old", "sib-new"])),
        (
            ORG_TABLE,
            "organization_id",
            "o-sib",
            Some(&["sib-old", "sib-new"]),
        ),
        (
            PROJECT_TABLE,
            "project_id",
            "p-sib",
            Some(&["sib-old", "sib-new"]),
        ),
        (
            USER_TABLE,
            "user_id",
            "u-sib",
            Some(&["sib-old", "sib-new"]),
        ),
    ],
    keys: &[
        "team_id:t-sib",
        "k-sib",
        "org_id:o-sib",
        "org_id:o-sib:with_budget",
        "project_id:p-sib",
        "u-sib",
    ],
};

const CONFIG_SIBLING: PinnedCase = PinnedCase {
    name: "config_sibling",
    deployments: &[],
    teams: &[(Some("t-cfg"), Some("cfg-alias"), Some(&["cfg-old"]))],
    allowlists: &[(USER_TABLE, "user_id", "u-cfg", Some(&["cfg-old"]))],
    catalog: &[("cfg-old", "dep-cfg")],
    model_id: "dep-1",
    old_name: "cfg-old",
    new_name: "cfg-new",
    stored: &[
        (
            TEAM_TABLE,
            "team_id",
            "t-cfg",
            Some(&["cfg-old", "cfg-new"]),
        ),
        (
            USER_TABLE,
            "user_id",
            "u-cfg",
            Some(&["cfg-old", "cfg-new"]),
        ),
    ],
    keys: &["team_id:t-cfg", "team_alias:cfg-alias", "u-cfg"],
};

const LAST_SIBLING: PinnedCase = PinnedCase {
    name: "last_sibling",
    deployments: &[("dep-last", "last-new")],
    teams: &[],
    allowlists: &[
        (
            PROJECT_TABLE,
            "project_id",
            "p-last",
            Some(&["last-old", "last-old", "z"]),
        ),
        (
            ORG_TABLE,
            "organization_id",
            "o-last",
            Some(&["last-new", "last-old"]),
        ),
    ],
    catalog: &[],
    model_id: "dep-last",
    old_name: "last-old",
    new_name: "last-new",
    stored: &[
        (
            PROJECT_TABLE,
            "project_id",
            "p-last",
            Some(&["last-new", "last-new", "z"]),
        ),
        (ORG_TABLE, "organization_id", "o-last", Some(&["last-new"])),
    ],
    keys: &[
        "project_id:p-last",
        "org_id:o-last",
        "org_id:o-last:with_budget",
    ],
};

const DUPLICATE_OLD: PinnedCase = PinnedCase {
    name: "duplicate_old",
    deployments: &[],
    teams: &[],
    allowlists: &[(
        KEY_TABLE,
        "token",
        "k-dup",
        Some(&["dup-old", "x", "dup-old"]),
    )],
    catalog: &[],
    model_id: "dep-1",
    old_name: "dup-old",
    new_name: "dup-new",
    stored: &[(
        KEY_TABLE,
        "token",
        "k-dup",
        Some(&["dup-new", "x", "dup-new"]),
    )],
    keys: &["k-dup"],
};

const EXISTING_NEW: PinnedCase = PinnedCase {
    name: "existing_new",
    deployments: &[
        ("dep-exist", "exist-new"),
        ("dep-exist-sibling", "exist-old"),
    ],
    teams: &[],
    allowlists: &[
        (
            KEY_TABLE,
            "token",
            "k-exist-both",
            Some(&["exist-new", "exist-old"]),
        ),
        (KEY_TABLE, "token", "k-exist-append", Some(&["exist-old"])),
    ],
    catalog: &[],
    model_id: "dep-exist",
    old_name: "exist-old",
    new_name: "exist-new",
    stored: &[
        (
            KEY_TABLE,
            "token",
            "k-exist-both",
            Some(&["exist-new", "exist-old"]),
        ),
        (
            KEY_TABLE,
            "token",
            "k-exist-append",
            Some(&["exist-old", "exist-new"]),
        ),
    ],
    keys: &["k-exist-append"],
};

const NO_OLD: PinnedCase = PinnedCase {
    name: "no_old",
    deployments: &[],
    teams: &[],
    allowlists: &[(USER_TABLE, "user_id", "u-no-old", Some(&["unrelated"]))],
    catalog: &[],
    model_id: "dep-1",
    old_name: "no-old",
    new_name: "no-new",
    stored: &[(USER_TABLE, "user_id", "u-no-old", Some(&["unrelated"]))],
    keys: &[],
};

const NULL_ARRAY: PinnedCase = PinnedCase {
    name: "null_array",
    deployments: &[],
    teams: &[(Some("t-null"), None, None)],
    allowlists: &[(KEY_TABLE, "token", "k-null", None)],
    catalog: &[],
    model_id: "dep-1",
    old_name: "null-old",
    new_name: "null-new",
    stored: &[
        (TEAM_TABLE, "team_id", "t-null", None),
        (KEY_TABLE, "token", "k-null", None),
    ],
    keys: &[],
};

const EMPTY_ARRAY: PinnedCase = PinnedCase {
    name: "empty_array",
    deployments: &[],
    teams: &[],
    allowlists: &[(USER_TABLE, "user_id", "u-empty", Some(&[]))],
    catalog: &[],
    model_id: "dep-1",
    old_name: "empty-old",
    new_name: "empty-new",
    stored: &[(USER_TABLE, "user_id", "u-empty", Some(&[]))],
    keys: &[],
};

const LITERAL_WILDCARD: PinnedCase = PinnedCase {
    name: "literal_wildcard",
    deployments: &[],
    teams: &[],
    allowlists: &[
        (PROJECT_TABLE, "project_id", "p-wild-new", Some(&["gpt-4o"])),
        (
            PROJECT_TABLE,
            "project_id",
            "p-wild-literal",
            Some(&["gpt-*"]),
        ),
    ],
    catalog: &[],
    model_id: "dep-1",
    old_name: "gpt-*",
    new_name: "gpt-4o",
    stored: &[
        (PROJECT_TABLE, "project_id", "p-wild-new", Some(&["gpt-4o"])),
        (
            PROJECT_TABLE,
            "project_id",
            "p-wild-literal",
            Some(&["gpt-4o"]),
        ),
    ],
    keys: &["project_id:p-wild-literal"],
};

const QUOTED_NAME: PinnedCase = PinnedCase {
    name: "quoted_name",
    deployments: &[],
    teams: &[],
    allowlists: &[
        (KEY_TABLE, "token", "k-quote", Some(&["it's \"quoted\" $1"])),
        (KEY_TABLE, "token", "k-quote-other", Some(&["it"])),
    ],
    catalog: &[],
    model_id: "dep-1",
    old_name: "it's \"quoted\" $1",
    new_name: "new 'name'",
    stored: &[
        (KEY_TABLE, "token", "k-quote", Some(&["new 'name'"])),
        (KEY_TABLE, "token", "k-quote-other", Some(&["it"])),
    ],
    keys: &["k-quote"],
};

fn to_array(models: AllowlistModels) -> Option<Vec<String>> {
    models.map(expected)
}

async fn run_pinned_case(pool: &Pool, case: &PinnedCase) {
    for (model_id, model_name) in case.deployments {
        insert_model(pool, model_id, model_name).await;
    }
    for (team_id, team_alias, models) in case.teams {
        let values = to_array(*models);
        insert_team(pool, *team_id, *team_alias, values.as_deref()).await;
    }
    for (table, id_column, id, models) in case.allowlists {
        let values = to_array(*models);
        insert_allowlist(pool, table, id_column, id, values.as_deref()).await;
    }
    let router_catalog = if case.catalog.is_empty() {
        None
    } else {
        Some(catalog(case.catalog))
    };
    let harness = Harness::new(pool);
    let cache = harness.cache();
    let rename = request(case.model_id, case.old_name, case.new_name);
    harness
        .run(&rename, &cache, router_catalog.as_ref())
        .await
        .unwrap();
    for (table, id_column, id, models) in case.stored {
        let name = case.name;
        assert_eq!(
            stored(pool, table, id_column, id).await,
            to_array(*models),
            "the {name} case"
        );
    }
    assert_keys(&cache, case.keys);
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
    assert!(harness.events.lock().expect("events").is_empty());
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
    let recorded = harness.events.lock().expect("events").clone();
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
