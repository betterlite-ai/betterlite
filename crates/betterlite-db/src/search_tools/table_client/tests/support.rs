use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use betterlite_postgres_fixture::connection::{PostgresFixture, RandomDatabaseName};
use betterlite_postgres_fixture::seed::FixtureSeed;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use clap::Parser;
use deadpool_postgres::{GenericClient as _, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::postgres_store::session::PostgresStore;
use crate::schema_managed_resources::ADD_SEARCH_TOOL_TABLE;

pub(crate) type Journal = Arc<Mutex<Vec<String>>>;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn test_url() -> Option<SecretString> {
    Environment::parse_from(["search-tools"])
        .url
        .map(SecretString::from)
}

// Each test owns a database, so tests never see one another's rows
async fn provision() -> PostgresFixture {
    let mut generator = RandomDatabaseName;
    let mut task = PostgresFixture::provision_with_name_generator(
        test_url().as_ref(),
        &FixtureSeed::default(),
        &mut generator,
    );
    task.resolve().await.unwrap()
}

pub(crate) struct Case {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    availability: Availability,
}

impl Case {
    pub(crate) async fn new() -> Self {
        let fixture = provision().await;
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(2)
            .build()
            .unwrap();
        let store = PostgresStore { pool: Some(pool) };
        let client = store.session().await.unwrap();
        client
            .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
            .await
            .unwrap();
        client
            .batch_execute(from_utf8(ADD_SEARCH_TOOL_TABLE.bytes).unwrap())
            .await
            .unwrap();
        drop(client);
        Self {
            fixture,
            store,
            availability: Availability::default(),
        }
    }

    pub(crate) const fn client(&self) -> PrismaClient<'_> {
        PrismaClient {
            writer: &self.store,
            reader: &self.store,
            availability: &self.availability,
        }
    }

    pub(crate) async fn close(self) {
        self.store.pool.unwrap().close();
        self.fixture.close_async().await.unwrap();
    }
}

fn publish_result(result: Result<(), TableError>) -> PublishFuture<TableError> {
    Box::pin(async move { result })
}

pub(crate) fn recording_publisher(journal: &Journal) -> PublishChange<TableError> {
    let entries = Arc::clone(journal);
    Arc::new(move |object_type: &str| {
        entries
            .lock()
            .unwrap()
            .push(format!("publish:{object_type}"));
        publish_result(Ok(()))
    })
}

pub(crate) fn failing_publisher() -> PublishChange<TableError> {
    Arc::new(|_object_type: &str| {
        publish_result(Err(TableError::Publish {
            message: "redis is down".to_owned(),
        }))
    })
}

pub(crate) async fn row_count(store: &PostgresStore) -> i64 {
    let client = store.session().await.unwrap();
    let row = client
        .query_one(
            "SELECT count(*) AS total FROM \"LiteLLM_SearchToolsTable\"",
            &[],
        )
        .await
        .unwrap();
    row.get("total")
}
