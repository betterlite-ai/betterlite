use betterlite_db::auxiliary_repository::repository::{PrismaClient, TableError};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

pub(crate) const INSERT_ROW: &str = "INSERT INTO \"LiteLLM_ProxyModelTable\" \
    (model_id, model_name, litellm_params, model_info) VALUES ($1, $2, '{}'::jsonb, $3::text::jsonb)";

pub(crate) const INSERT_SCANNED_ROW: &str = "INSERT INTO \"WeightRows\" \
    (model_id, model_name, litellm_params, model_info) VALUES ($1, $2, '{}'::jsonb, $3::text::jsonb)";

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Case {
    pub(crate) fixture: PostgresFixture,
    pub(crate) pool: Pool,
    pub(crate) store: PostgresStore,
    pub(crate) availability: Availability,
}

impl Case {
    pub(crate) async fn new(schema: &str) -> Self {
        let url = Environment::parse_from(["router-weight-references-tests"])
            .url
            .map(SecretString::from);
        let fixture =
            PostgresFixture::provision_with_statements(url.as_ref(), vec![schema.to_owned()])
                .resolve()
                .await
                .unwrap();
        let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
        let pool = Pool::builder(manager).max_size(2).build().unwrap();
        Self {
            fixture,
            store: PostgresStore {
                pool: Some(pool.clone()),
            },
            pool,
            availability: Availability::default(),
        }
    }

    pub(crate) fn repository(&self) -> ModelRepository<'_> {
        repository_for(&self.store, &self.availability)
    }

    pub(crate) async fn insert(
        &self,
        statement: &str,
        model_id: &str,
        model_name: &str,
        model_info: Option<&str>,
    ) {
        let client = self.pool.get().await.unwrap();
        client
            .execute(statement, &[&model_id, &model_name, &model_info])
            .await
            .unwrap();
    }

    pub(crate) async fn count(&self, sql: &str) -> i64 {
        let client = self.pool.get().await.unwrap();
        client.query_one(sql, &[]).await.unwrap().get(0)
    }

    pub(crate) async fn texts(&self, sql: &str) -> Vec<String> {
        let client = self.pool.get().await.unwrap();
        client
            .query(sql, &[])
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    pub(crate) async fn close(self) {
        self.pool.close();
        self.fixture.close_async().await.unwrap();
    }
}

pub(crate) struct MissingDatabase {
    store: PostgresStore,
    availability: Availability,
}

impl MissingDatabase {
    pub(crate) fn new() -> Self {
        Self {
            store: PostgresStore { pool: None },
            availability: Availability::default(),
        }
    }

    pub(crate) fn repository(&self) -> ModelRepository<'_> {
        repository_for(&self.store, &self.availability)
    }
}

pub(crate) fn repository_for<'db>(
    store: &'db PostgresStore,
    availability: &'db Availability,
) -> ModelRepository<'db> {
    let client = PrismaClient {
        writer: store,
        reader: store,
        availability,
    };
    ModelRepository::new(
        Some(client),
        redis_config_change_publisher::<TableError>(None),
        None,
        cipher(),
    )
}

fn cipher() -> DbCipher {
    DbCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(
                "router-weight-references-key".to_owned(),
            )),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
