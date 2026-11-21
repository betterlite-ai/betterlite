use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::aliases::ModelAliases;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;
use tokio_util::task::TaskTracker;

use crate::auth_read::table::{
    AuthReadError, AuthTable as _, Create, FindMany, FindOne, Presence, Update,
};
use crate::auth_team_lookup::ports::{TeamStore as _, TeamStoreError};
use crate::auth_team_lookup::store::{DatabaseTeamStore, LookupTable, RowKey};

const SEED: &str = "INSERT INTO \"LiteLLM_ModelTable\" (id, aliases, created_by, updated_by) VALUES (7, '{\"fast\": \"gpt-4o\"}', 'a', 'a'); INSERT INTO \"LiteLLM_TeamTable\" (team_id, team_alias, model_id) VALUES ('t1', 'solo', 7), ('t2', 'shared', NULL), ('t3', 'shared', NULL); INSERT INTO \"LiteLLM_AccessGroupTable\" (access_group_id, access_group_name) VALUES ('g1', 'group one')";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn provision() -> PostgresFixture {
    let url = Environment::parse_from(["auth-team-lookup-postgres-tests"])
        .url
        .map(SecretString::from);
    let statements = catalog_statements(&PACKAGED_CATALOGS).unwrap();
    PostgresFixture::provision_with_statements(url.as_ref(), statements)
        .resolve()
        .await
        .unwrap()
}

fn pool_for(fixture: &PostgresFixture) -> Pool {
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    Pool::builder(manager).max_size(4).build().unwrap()
}

fn team_store(reader: PostgresStore, writer: PostgresStore) -> DatabaseTeamStore {
    DatabaseTeamStore {
        reader,
        writer,
        availability: Arc::new(Availability::default()),
        default_deadline_seconds: 5.0_f64,
        tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        tasks: TaskTracker::new(),
    }
}

async fn assert_teams(database: &DatabaseTeamStore) {
    let row = database.find_team("t1", false).await.unwrap().unwrap();
    let team = LiteLLMTeamTableCachedObj::try_from(row).unwrap();
    assert!(
        team.team.fields.base.models.is_empty(),
        "a null text[] reads as []"
    );
    let relation = team.team.fields.litellm_model_table.unwrap();
    assert_eq!(relation.id, Some(7));
    assert!(
        matches!(&relation.model_aliases, Some(ModelAliases::Map(aliases)) if aliases.contains_key("fast"))
    );
    assert_eq!(database.find_team("absent", false).await.unwrap(), None);

    let shared = database.find_by_alias("shared").await.unwrap();
    assert_eq!(shared.len(), 2);
    for shared_row in shared {
        let member = LiteLLMTeamTableCachedObj::try_from(shared_row).unwrap();
        assert_eq!(member.team.fields.litellm_model_table, None);
    }
    assert_eq!(database.find_by_alias("solo").await.unwrap().len(), 1);
    assert!(database.find_by_alias("nobody").await.unwrap().is_empty());
}

async fn assert_group(database: &DatabaseTeamStore) {
    let row = database
        .find_access_group("g1", false)
        .await
        .unwrap()
        .unwrap();
    let group: AccessGroupTable =
        serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
    assert_eq!(group.access_group_name, "group one");
    assert!(group.access_model_names.is_empty());
    assert_eq!(
        database.find_access_group("absent", false).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn relation_projection() {
    let fixture = provision().await;
    let store = PostgresStore {
        pool: Some(pool_for(&fixture)),
    };
    store
        .session()
        .await
        .unwrap()
        .batch_execute(SEED)
        .await
        .unwrap();
    let database = team_store(store.clone(), store.clone());
    assert_teams(&database).await;
    assert_group(&database).await;

    let writer_only = team_store(PostgresStore { pool: None }, store.clone());
    assert!(writer_only.find_team("t1", true).await.unwrap().is_some());
    let refused = writer_only.find_team("t1", false).await.unwrap_err();
    assert!(
        matches!(refused, TeamStoreError::Read(_)),
        "the reader handle is not connected"
    );
    assert!(
        writer_only
            .find_access_group("g1", true)
            .await
            .unwrap()
            .is_some()
    );

    database.tasks.close();
    database.tasks.wait().await;
    writer_only.tasks.close();
    writer_only.tasks.wait().await;
    if let Some(pool) = store.pool.as_ref() {
        pool.close();
    }
    fixture.close_async().await.unwrap();
}

fn unread<Row>(answer: &Result<Row, AuthReadError<Infallible>>) -> bool {
    matches!(answer, Err(AuthReadError::Repository(DbError::CreateNull)))
}

#[tokio::test]
async fn unwired_actions_refuse() {
    let table = LookupTable {
        store: PostgresStore { pool: None },
    };
    let first = FindOne {
        filter: "t1".to_owned(),
        include: Presence::Omitted,
    };
    assert!(unread(&table.find_first(first).await));
    let many = FindMany {
        filter: Presence::Omitted,
        include: Presence::Omitted,
        take: Presence::Omitted,
    };
    assert!(unread(&table.find_many(many).await));
    let update = Update {
        filter: RowKey::Team("t1".to_owned()),
        data: (),
    };
    assert!(unread(&table.update(update).await));
    let create = Create {
        data: (),
        include: Presence::Omitted,
    };
    assert!(unread(&table.create(create).await));
}
