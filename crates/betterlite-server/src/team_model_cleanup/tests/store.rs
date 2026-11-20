use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_repository::decoder::decode_team_record;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_postgres_fixture::sql::BASELINE;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::team_model_cleanup::alias::{AliasRow, AliasRows as _};
use crate::team_model_cleanup::deployments::DeploymentRows as _;
use crate::team_model_cleanup::error::CleanupError;
use crate::team_model_cleanup::remove::TeamModelRows as _;
use crate::team_model_cleanup::store::PostgresTeamModels;

use super::support::{aliases, names, value};

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, Pool, Client) {
    let url = Environment::parse_from(["team-model-cleanup-tests"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(url.as_ref(), vec![BASELINE.to_owned()])
            .resolve()
            .await
            .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    (fixture, pool, client)
}

fn store(pool: &Pool) -> (PostgresStore, Availability) {
    (
        PostgresStore {
            pool: Some(pool.clone()),
        },
        Availability::default(),
    )
}

fn cleanup<'db>(
    database: &'db PostgresStore,
    availability: &'db Availability,
) -> PostgresTeamModels<'db> {
    PostgresTeamModels {
        client: Some(PrismaClient {
            writer: database,
            reader: database,
            availability,
        }),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

const INSERT_ALIASES: &str = r#"INSERT INTO "LiteLLM_ModelTable" (id, aliases, created_by, updated_by)
    VALUES ($1, $2::TEXT::JSONB, 'creator', 'editor')"#;

async fn insert_aliases(client: &Client, id: i32, stored: Option<&str>) {
    client
        .execute(INSERT_ALIASES, &[&id, &stored])
        .await
        .unwrap();
}

async fn insert_team(
    client: &Client,
    team_id: &str,
    team_models: Option<&[String]>,
    model_id: Option<i32>,
) {
    client
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id, models, model_id) VALUES ($1, $2, $3)"#,
            &[&team_id, &team_models, &model_id],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn alias_rows_reads_the_team_relation_and_the_stored_object() {
    let (fixture, pool, client) = database().await;
    insert_aliases(&client, 1, Some(r#"{"a":"pub"}"#)).await;
    insert_aliases(&client, 2, None).await;
    insert_aliases(&client, 3, Some(r#"{"b":"other"}"#)).await;
    insert_team(&client, "t1", Some(&names(&["pub"])), Some(1)).await;
    insert_team(&client, "t2", Some(&names(&["other"])), Some(2)).await;
    drop(client);
    let (stored, availability) = store(&pool);
    let found = cleanup(&stored, &availability).alias_rows().await.unwrap();
    assert_eq!(
        found,
        vec![
            AliasRow {
                id: 1,
                team_id: Some("t1".to_owned()),
                aliases: Some(aliases(r#"{"a":"pub"}"#)),
            },
            AliasRow {
                id: 2,
                team_id: Some("t2".to_owned()),
                aliases: None,
            },
            AliasRow {
                id: 3,
                team_id: None,
                aliases: Some(aliases(r#"{"b":"other"}"#)),
            },
        ]
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn save_aliases_replaces_the_stored_object() {
    let (fixture, pool, client) = database().await;
    insert_aliases(&client, 7, Some(r#"{"old":"pub"}"#)).await;
    drop(client);
    let (stored, availability) = store(&pool);
    let cleanup_models = cleanup(&stored, &availability);
    cleanup_models
        .save_aliases(7, &aliases(r#"{"new":"kept"}"#))
        .await
        .unwrap();
    let found = cleanup_models.alias_rows().await.unwrap();
    assert_eq!(found[0].aliases, Some(aliases(r#"{"new":"kept"}"#)));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_non_object_alias_column_is_an_error() {
    let (fixture, pool, client) = database().await;
    insert_aliases(&client, 9, Some("[1]")).await;
    drop(client);
    let (stored, availability) = store(&pool);
    let error = cleanup(&stored, &availability)
        .alias_rows()
        .await
        .expect_err("a list is not an alias mapping");
    assert!(matches!(error, CleanupError::Aliases { id: 9 }));
    close(fixture, pool).await;
}

#[tokio::test]
async fn deployments_with_prefix_returns_the_matching_rows() {
    let (fixture, pool, client) = database().await;
    client
        .batch_execute(
            r#"INSERT INTO "LiteLLM_ProxyModelTable"
                (model_id, model_name, litellm_params, model_info, created_by, updated_by)
                VALUES
                ('id-a', 'model_name_t1_a', '{}', '{"team_id":"t1"}', 'creator', 'editor'),
                ('id-b', 'model_name_t10_b', '{}', '{"team_id":"t1"}', 'creator', 'editor'),
                ('id-c', 'model_name_t1_c', '{}', NULL, 'creator', 'editor')"#,
        )
        .await
        .unwrap();
    drop(client);
    let (stored, availability) = store(&pool);
    let found = cleanup(&stored, &availability)
        .deployments_with_prefix("model_name_t1_")
        .await
        .unwrap();
    let found_names: Vec<&str> = found.iter().map(|row| row.model_name.as_str()).collect();
    assert_eq!(found_names, vec!["model_name_t1_a", "model_name_t1_c"]);
    assert_eq!(found[0].model_info, value(r#"{"team_id":"t1"}"#));
    assert_eq!(found[1].model_info, value("null"));
    close(fixture, pool).await;
}

#[tokio::test]
async fn team_models_reads_the_row_or_none() {
    let (fixture, pool, client) = database().await;
    insert_team(&client, "t1", Some(&names(&["a", "b"])), None).await;
    client
        .execute(
            r#"INSERT INTO "LiteLLM_TeamTable" (team_id, models) VALUES ('t3', NULL)"#,
            &[],
        )
        .await
        .unwrap();
    drop(client);
    let (stored, availability) = store(&pool);
    let cleanup_models = cleanup(&stored, &availability);
    assert_eq!(
        cleanup_models.team_models("t1").await.unwrap(),
        Some(names(&["a", "b"]))
    );
    assert_eq!(
        cleanup_models.team_models("t3").await.unwrap(),
        Some(Vec::new())
    );
    assert_eq!(cleanup_models.team_models("absent").await.unwrap(), None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn save_team_models_returns_the_updated_row() {
    let (fixture, pool, client) = database().await;
    insert_team(&client, "t1", Some(&names(&["a", "b"])), None).await;
    drop(client);
    let (stored, availability) = store(&pool);
    let cleanup_models = cleanup(&stored, &availability);
    let record = cleanup_models
        .save_team_models("t1", &names(&["b"]))
        .await
        .unwrap();
    let decoded = decode_team_record(Some(&record)).unwrap().unwrap();
    assert_eq!(decoded.fields.base.team_id, "t1");
    assert_eq!(decoded.fields.base.models, names(&["b"]));
    assert_eq!(
        cleanup_models.team_models("t1").await.unwrap(),
        Some(names(&["b"]))
    );
    let error = cleanup_models
        .save_team_models("absent", &names(&["b"]))
        .await
        .expect_err("an absent team row cannot be updated");
    assert!(matches!(error, CleanupError::Team(_)));
    close(fixture, pool).await;
}

#[tokio::test]
async fn a_saved_alias_without_the_client_is_an_error() {
    let availability = Availability::default();
    let absent = PostgresStore { pool: None };
    let cleanup_models = PostgresTeamModels {
        client: Some(PrismaClient {
            writer: &absent,
            reader: &absent,
            availability: &availability,
        }),
    };
    let error = cleanup_models
        .alias_rows()
        .await
        .expect_err("no pool is an error");
    assert!(matches!(error, CleanupError::Database(_)));
    let disconnected = PostgresTeamModels { client: None };
    let error = disconnected
        .team_models("t1")
        .await
        .expect_err("no client is an error");
    assert!(matches!(error, CleanupError::Database(_)));
}
