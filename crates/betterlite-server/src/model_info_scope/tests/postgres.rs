use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_models::decisions::Json;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_group_lookups::access_groups::AccessGroups;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_postgres::types::ToSql;

use super::{caller, identified, ids, snapshot, team_list, team_row};
use crate::model_info_scope::compose::{
    TeamSelection, check_if_model_is_user_added, get_all_team_models, non_admin_all_models,
    populate_team_access_on_models,
};
use crate::model_info_scope::error::{ScopeDetail, ScopeError, USER_NOT_FOUND};
use crate::model_info_scope::postgres::PostgresScopeReads;

// The columns these reads touch, as LiteLLM's migrations declare them: every list column is nullable.
const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_ProxyModelTable" (
    model_id TEXT PRIMARY KEY,
    model_name TEXT NOT NULL,
    litellm_params JSONB NOT NULL,
    model_info JSONB,
    created_by TEXT
);
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    teams JSONB,
    models JSONB
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    admins JSONB,
    members JSONB,
    models JSONB,
    access_group_ids JSONB
);
CREATE TABLE "LiteLLM_AccessGroupTable" (
    access_group_id TEXT PRIMARY KEY,
    access_group_name TEXT NOT NULL,
    access_model_names JSONB,
    access_mcp_server_ids JSONB,
    access_agent_ids JSONB,
    assigned_team_ids JSONB,
    assigned_key_ids JSONB
);
"#;

const INSERT_MODEL_SQL: &str = r#"INSERT INTO "LiteLLM_ProxyModelTable" (model_id, model_name, litellm_params, created_by) VALUES ($1, 'gpt-4o', '{"model":"openai/gpt-4o"}'::jsonb, $2)"#;
const INSERT_USER_SQL: &str = r#"INSERT INTO "LiteLLM_UserTable" (user_id, teams, models) VALUES ($1, ($2::text)::jsonb, ($3::text)::jsonb)"#;
const INSERT_TEAM_SQL: &str = r#"INSERT INTO "LiteLLM_TeamTable" (team_id, models, access_group_ids) VALUES ($1, ($2::text)::jsonb, ($3::text)::jsonb)"#;
const INSERT_GROUP_SQL: &str = r#"INSERT INTO "LiteLLM_AccessGroupTable" (access_group_id, access_group_name, access_model_names) VALUES ($1, $1, ($2::text)::jsonb)"#;
const SET_TEAM_MODELS_SQL: &str =
    r#"UPDATE "LiteLLM_TeamTable" SET models = ($1::text)::jsonb WHERE team_id = $2"#;
const DROP_USER_SQL: &str = r#"DROP TABLE "LiteLLM_UserTable""#;
const DROP_GROUPS_SQL: &str = r#"DROP TABLE "LiteLLM_AccessGroupTable""#;

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
        let url = Environment::parse_from(["model-info-scope-tests"])
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

    fn reads(&self) -> PostgresScopeReads<'_> {
        PostgresScopeReads {
            client: PrismaClient {
                writer: &self.store,
                reader: &self.store,
                availability: &self.availability,
            },
        }
    }

    async fn execute(&self, statement: &str, parameters: &[&(dyn ToSql + Sync)]) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .execute(statement, parameters)
            .await
            .expect("the statement runs");
    }

    async fn batch(&self, statement: &str) {
        let client = self.pool.get().await.expect("a fixture connection");
        client
            .batch_execute(statement)
            .await
            .expect("the statements run");
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture closes");
    }
}

fn json_list(values: &[&str]) -> String {
    let items: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    format!("[{}]", items.join(","))
}

async fn seed_creator(db: &Database, model_id: &str, created_by: Option<&str>) {
    db.execute(INSERT_MODEL_SQL, &[&model_id, &created_by])
        .await;
}

async fn seed_user(db: &Database, user_id: &str, teams: &[&str], models: &[&str]) {
    let teams = json_list(teams);
    let models = json_list(models);
    db.execute(INSERT_USER_SQL, &[&user_id, &teams, &models])
        .await;
}

async fn seed_team(db: &Database, team_id: &str, models: Option<&[&str]>, groups: Option<&[&str]>) {
    let models = models.map(json_list);
    let groups = groups.map(json_list);
    db.execute(INSERT_TEAM_SQL, &[&team_id, &models, &groups])
        .await;
}

async fn seed_group(db: &Database, group_id: &str, models: &[&str]) {
    let models = json_list(models);
    db.execute(INSERT_GROUP_SQL, &[&group_id, &models]).await;
}

#[tokio::test]
async fn creator_reads_by_id() {
    let db = Database::start().await;
    seed_creator(&db, "d1", Some("alice")).await;
    seed_creator(&db, "d2", Some("bob")).await;
    // d3 has no stored row and the last row has no id, so neither can match the caller.
    let rows = vec![
        identified("gpt-4o", "d1"),
        identified("gpt-4o-mini", "d2"),
        identified("gpt-4o-small", "d3"),
        super::unnumbered("no-id-gpt", Vec::new()),
    ];
    let lookup = snapshot(&rows[..3]);
    let kept = non_admin_all_models(
        rows,
        &caller(Some("alice"), &[]),
        Some(&db.reads()),
        &lookup.lookup(),
    )
    .await
    .unwrap();
    assert_eq!(ids(&kept), vec![Some("d1")]);
    db.close().await;
}

#[tokio::test]
async fn no_database() {
    let empty: Vec<DeploymentSpec> = Vec::new();
    let lookup = snapshot(&empty);
    let anonymous = caller(None, &[]);
    let error = check_if_model_is_user_added(empty.clone(), &anonymous, None)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        error.detail(),
        ScopeDetail::Error {
            error: CommonProxyErrors::DbNotConnectedError.as_str(),
        }
    );
    let second = non_admin_all_models(empty, &anonymous, None, &lookup.lookup())
        .await
        .unwrap_err();
    assert_eq!(second.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn user_read_error_400() {
    let db = Database::start().await;
    seed_creator(&db, "d1", Some("alice")).await;
    db.batch(DROP_USER_SQL).await;
    let rows = vec![identified("gpt-4o", "d1")];
    let lookup = snapshot(&rows);
    let error = non_admin_all_models(
        rows,
        &caller(Some("alice"), &[]),
        Some(&db.reads()),
        &lookup.lookup(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        error.detail(),
        ScopeDetail::Error {
            error: USER_NOT_FOUND,
        }
    );
    db.close().await;
}

#[tokio::test]
async fn missing_user() {
    let db = Database::start().await;
    seed_creator(&db, "d1", Some("alice")).await;
    let rows = vec![
        identified("gpt-4o", "d1"),
        team_row("team-gpt", "t1", "team-alpha"),
    ];
    let lookup = snapshot(&rows);
    let kept = non_admin_all_models(
        rows,
        &caller(Some("alice"), &[]),
        Some(&db.reads()),
        &lookup.lookup(),
    )
    .await
    .unwrap();
    assert_eq!(ids(&kept), vec![Some("d1")]);
    db.close().await;
}

#[tokio::test]
async fn team_batch_query() {
    let db = Database::start().await;
    seed_team(&db, "team-alpha", Some(&["gpt-4o"]), None).await;
    seed_team(&db, "team-beta", Some(&["gpt-4o"]), None).await;
    seed_team(&db, "team-gamma", Some(&["gpt-4o-mini"]), None).await;
    let rows = vec![identified("gpt-4o", "g1"), identified("gpt-4o-mini", "g2")];
    let lookup = snapshot(&rows);
    let selection = TeamSelection::Ids(team_list(&["team-alpha", "team-beta"]));
    let found = get_all_team_models(
        &selection,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    // Both selected teams reach g1 through one read, and the unselected team's model stays out.
    assert_eq!(
        found.get("g1"),
        Some(&team_list(&["team-alpha", "team-beta"]))
    );
    assert_eq!(found.get("g2"), None);
    db.close().await;
}

#[tokio::test]
async fn access_group_batch_query() {
    let db = Database::start().await;
    seed_team(&db, "team-alpha", Some(&["base"]), Some(&["ag-1", "ag-2"])).await;
    seed_team(&db, "team-beta", Some(&["base"]), Some(&["ag-2", "ag-3"])).await;
    seed_group(&db, "ag-1", &["group-model"]).await;
    seed_group(&db, "ag-2", &["group-model"]).await;
    seed_group(&db, "ag-3", &["other"]).await;
    let rows = vec![
        identified("base", "base-1"),
        identified("group-model", "group-1"),
        identified("other", "other-1"),
    ];
    let lookup = snapshot(&rows);
    let selection = TeamSelection::Ids(team_list(&["team-alpha", "team-beta"]));
    let found = get_all_team_models(
        &selection,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    let both = team_list(&["team-alpha", "team-beta"]);
    assert_eq!(found.get("base-1"), Some(&both));
    assert_eq!(found.get("group-1"), Some(&both));
    assert_eq!(found.get("other-1"), Some(&team_list(&["team-beta"])));
    db.close().await;
}

#[tokio::test]
async fn missing_access_group() {
    let db = Database::start().await;
    seed_team(
        &db,
        "team-alpha",
        Some(&["base"]),
        Some(&["ag-1", "missing"]),
    )
    .await;
    seed_group(&db, "ag-1", &["group-model"]).await;
    let rows = vec![
        identified("base", "base-1"),
        identified("group-model", "group-1"),
    ];
    let lookup = snapshot(&rows);
    let selection = TeamSelection::Ids(team_list(&["team-alpha"]));
    let found = get_all_team_models(
        &selection,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    assert_eq!(found.get("base-1"), Some(&team_list(&["team-alpha"])));
    assert_eq!(found.get("group-1"), Some(&team_list(&["team-alpha"])));
    db.close().await;
}

#[tokio::test]
async fn unrestricted_skips_groups() {
    let db = Database::start().await;
    seed_team(&db, "team-alpha", None, Some(&["ag-1"])).await;
    db.batch(DROP_GROUPS_SQL).await;
    let rows = vec![
        identified("gpt-4o", "g1"),
        team_row("team-gpt", "t2", "team-beta"),
        team_row("inner-gpt", "t3", "alpha"),
    ];
    let lookup = snapshot(&rows);
    let found = get_all_team_models(
        &TeamSelection::All,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    // A null models column is unrestricted, and the dropped table proves no group read follows.
    assert_eq!(found.len(), 2);
    assert_eq!(found.get("g1"), Some(&team_list(&["team-alpha"])));
    assert_eq!(found.get("t3"), Some(&team_list(&["team-alpha"])));
    assert_eq!(found.get("t2"), None);
    db.close().await;
}

#[tokio::test]
async fn row_validation_failure() {
    let db = Database::start().await;
    seed_team(&db, "team-alpha", None, None).await;
    let rows = vec![identified("gpt-4o", "g1")];
    let lookup = snapshot(&rows);
    let selection = TeamSelection::Ids(team_list(&["team-alpha"]));
    let found = get_all_team_models(
        &selection,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    assert_eq!(found.get("g1"), Some(&team_list(&["team-alpha"])));
    db.execute(SET_TEAM_MODELS_SQL, &[&"\"not-a-list\"", &"team-alpha"])
        .await;
    let error = get_all_team_models(
        &selection,
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::TeamRead(_)));
    db.close().await;
}

#[tokio::test]
async fn annotation_read_failure() {
    let db = Database::start().await;
    db.batch(DROP_USER_SQL).await;
    let rows = vec![identified("gpt-4o", "g1")];
    let lookup = snapshot(&rows);
    let error = populate_team_access_on_models(
        rows,
        &caller(Some("alice"), &[]),
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap_err();
    // The annotation path does not catch a user read error, so it surfaces as LiteLLM's 500.
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(matches!(error, ScopeError::UserRead(_)));
    db.close().await;
}

// A stored roster reaches its direct grant and its teams for the annotations, all through the database.
#[tokio::test]
async fn stored_roster_annotates_models() {
    let db = Database::start().await;
    seed_user(&db, "alice", &["team-alpha"], &["granted"]).await;
    seed_team(&db, "team-alpha", Some(&["team-model"]), None).await;
    let rows = vec![
        identified("granted", "d1"),
        identified("team-model", "d2"),
        identified("other", "d3"),
    ];
    let lookup = snapshot(&rows);
    let annotated = populate_team_access_on_models(
        rows,
        &caller(Some("alice"), &[]),
        &db.reads(),
        &lookup.lookup(),
        &AccessGroups::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        annotated[0].model_info.get("access_via_team_ids"),
        Some(&super::team_ids(&[]))
    );
    assert_eq!(
        annotated[0].model_info.get("direct_access"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        annotated[1].model_info.get("access_via_team_ids"),
        Some(&super::team_ids(&["team-alpha"]))
    );
    assert_eq!(
        annotated[1].model_info.get("direct_access"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        annotated[2].model_info.get("direct_access"),
        Some(&Json::Bool(false))
    );
    db.close().await;
}
