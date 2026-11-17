use std::sync::Arc;

use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::sso_mappings::load::{load_role_mappings, load_team_mappings};
use crate::sso_mappings::repository::{
    DatabaseSsoSettings, SSO_CONFIG_ROW_ID, SsoSettingsRows as _,
};
use crate::sso_types::records::TeamMappings;
use crate::user_roles::UserRole;

use super::support::{role_mapping, sources};

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_SSOConfig" (
    id TEXT PRIMARY KEY DEFAULT 'sso_config',
    sso_settings JSONB NOT NULL,
    created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
"#;

const STORED_ROLE_MAPPING: &str = r#"{"role_mappings": {"provider": "generic", "group_claim": "groups", "roles": {"proxy_admin": ["admins"]}}}"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["sso-mappings-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
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
    let client = pool.get().await.expect("the database connection");
    client
        .batch_execute(SCHEMA)
        .await
        .expect("the settings table");
    drop(client);
    (fixture, pool)
}

fn settings_store(pool: &Pool) -> DatabaseSsoSettings {
    DatabaseSsoSettings {
        store: Arc::new(PostgresStore {
            pool: Some(pool.clone()),
        }),
        availability: Availability::default(),
    }
}

async fn insert_row(pool: &Pool, settings: &str) {
    let client = pool.get().await.expect("the database connection");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_SSOConfig" (sso_settings) VALUES ($1::text::jsonb)"#,
            &[&settings],
        )
        .await
        .expect("the settings row");
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.expect("the fixture cleanup");
}

#[tokio::test]
async fn reads_the_stored_mapping_from_the_database() {
    let (fixture, pool) = database().await;
    insert_row(&pool, STORED_ROLE_MAPPING).await;
    let loaded = load_role_mappings(&settings_store(&pool), &sources(&[])).await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, vec!["admins"])]
        )))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_environment_override_wins_over_the_database() {
    let (fixture, pool) = database().await;
    insert_row(&pool, STORED_ROLE_MAPPING).await;
    let pairs = [
        (
            "GENERIC_ROLE_MAPPINGS_ROLES",
            "{'internal_user': ['users']}",
        ),
        ("GENERIC_ROLE_MAPPINGS_GROUP_CLAIM", "roles"),
    ];
    let loaded = load_role_mappings(&settings_store(&pool), &sources(&pairs)).await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "roles",
            None,
            vec![(UserRole::InternalUser, vec!["users"])]
        )))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_non_dictionary_override_keeps_the_database_mapping() {
    let (fixture, pool) = database().await;
    insert_row(&pool, STORED_ROLE_MAPPING).await;
    let pairs = [("GENERIC_ROLE_MAPPINGS_ROLES", "['proxy_admin']")];
    let loaded = load_role_mappings(&settings_store(&pool), &sources(&pairs)).await;
    assert_eq!(
        loaded,
        Ok(Some(role_mapping(
            "groups",
            None,
            vec![(UserRole::ProxyAdmin, vec!["admins"])]
        )))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_settings_row_is_the_sso_config_id() {
    let (fixture, pool) = database().await;
    insert_row(
        &pool,
        r#"{"team_mappings": {"team_ids_jwt_field": "teams"}}"#,
    )
    .await;
    let text = settings_store(&pool)
        .stored_sso_settings()
        .await
        .expect("the row read")
        .expect("the settings row");
    assert_eq!(
        text,
        r#"{"team_mappings": {"team_ids_jwt_field": "teams"}}"#
    );
    assert_eq!(SSO_CONFIG_ROW_ID, "sso_config");
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_row_under_another_id_is_not_the_settings_row() {
    let (fixture, pool) = database().await;
    let client = pool.get().await.expect("the database connection");
    client
        .execute(
            r#"INSERT INTO "LiteLLM_SSOConfig" (id, sso_settings) VALUES ('other', $1::text::jsonb)"#,
            &[&STORED_ROLE_MAPPING],
        )
        .await
        .expect("the other row");
    let loaded = load_role_mappings(&settings_store(&pool), &sources(&[])).await;
    assert_eq!(loaded, Ok(None));
    close(fixture, pool).await;
}

#[tokio::test]
async fn an_absent_row_leaves_both_mappings_absent() {
    let (fixture, pool) = database().await;
    assert_eq!(load_team_mappings(&settings_store(&pool)).await, None);
    assert_eq!(
        load_role_mappings(&settings_store(&pool), &sources(&[])).await,
        Ok(None)
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn the_team_mapping_is_read_from_the_row() {
    let (fixture, pool) = database().await;
    insert_row(
        &pool,
        r#"{"team_mappings": {"team_ids_jwt_field": "teams"}}"#,
    )
    .await;
    let loaded = load_team_mappings(&settings_store(&pool)).await;
    assert_eq!(
        loaded,
        Some(TeamMappings {
            team_ids_jwt_field: Some("teams".to_owned())
        })
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_settings_value_that_is_not_an_object_leaves_the_mappings_absent() {
    let (fixture, pool) = database().await;
    insert_row(&pool, "[]").await;
    assert_eq!(load_team_mappings(&settings_store(&pool)).await, None);
    assert_eq!(
        load_role_mappings(&settings_store(&pool), &sources(&[])).await,
        Ok(None)
    );
    close(fixture, pool).await;
}
