use std::convert::Infallible;
use std::time::Duration;

use betterlite_db::database_refresh::handle::{PoolFuture, PoolTransport};
use betterlite_db::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::key_user_writes::retry::SystemWriteClock;
use betterlite_db::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use betterlite_db::migration_ledger::lock::JitterSource;
use betterlite_db::table_ops::{
    ConfigParamInvalidator, FailureFuture, FailureSink, InvalidateFuture, TableFailureEvent,
    TableOps, TablePorts, TableSettings,
};
use betterlite_db::tool_spend::TokioPause;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser as _;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::support::json;
use crate::config_param_cache::{ConfigParamStore, TableConfigParamStore};

#[derive(clap::Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

struct TableClient {
    pool: Pool,
}

impl DatabaseLifecycle for TableClient {
    type Error = Infallible;

    fn is_connected(&self) -> bool {
        true
    }

    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(false) })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for TableClient {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        let known = ["litellm_config", "litellm_usertable"];
        Ok(known
            .contains(&name)
            .then(|| Attribute::Model(self.pool.clone())))
    }

    fn token_refresh_enabled(&self) -> bool {
        false
    }

    fn token_label(&self) -> &'static str {
        "database token"
    }

    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}

struct ZeroJitter;

impl JitterSource for ZeroJitter {
    fn uniform_nanos(&self, _upper_exclusive_nanos: u64) -> u64 {
        0
    }
}

#[derive(Default)]
struct SilentSink;

impl FailureSink<Infallible> for SilentSink {
    fn record(&self, _event: TableFailureEvent<'_, Infallible>) -> FailureFuture<'_> {
        Box::pin(async {})
    }
}

#[derive(Default)]
struct NoopInvalidator;

impl ConfigParamInvalidator for NoopInvalidator {
    fn invalidate_param<'call>(&'call self, _param_name: &'call str) -> InvalidateFuture<'call> {
        Box::pin(async {})
    }
}

struct Ports {
    clock: SystemWriteClock,
    jitter: ZeroJitter,
    pause: TokioPause,
    sink: SilentSink,
    invalidator: NoopInvalidator,
}

impl Ports {
    fn new() -> Self {
        Self {
            clock: SystemWriteClock,
            jitter: ZeroJitter,
            pause: TokioPause,
            sink: SilentSink,
            invalidator: NoopInvalidator,
        }
    }

    fn table_ports(&self) -> TablePorts<'_, Infallible> {
        TablePorts {
            clock: &self.clock,
            jitter: &self.jitter,
            pause: &self.pause,
            sink: &self.sink,
            invalidator: &self.invalidator,
        }
    }
}

struct StoreHarness {
    fixture: PostgresFixture,
    pool: Pool,
    client: TableClient,
    ports: Ports,
}

impl StoreHarness {
    async fn start() -> Self {
        let configured = Environment::parse_from(["config-param-store-tests"])
            .url
            .map(SecretString::from);
        let statements = catalog_statements(&PACKAGED_CATALOGS).expect("the catalogs are UTF-8");
        let fixture = PostgresFixture::provision_with_statements(configured.as_ref(), statements)
            .resolve()
            .await
            .expect("a Postgres database fixture");
        let config = fixture
            .options
            .url
            .expose_secret()
            .parse()
            .expect("the fixture URL parses");
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .expect("the fixture pool builds");
        Self {
            fixture,
            client: TableClient { pool: pool.clone() },
            pool,
            ports: Ports::new(),
        }
    }

    fn operations(&self) -> TableOps<'_, TableClient> {
        TableOps {
            database: Some(WriterPinnedClient::Writer(&self.client)),
            ports: self.ports.table_ports(),
            settings: TableSettings::default(),
        }
    }

    async fn seed_config(&self, name: &str, value: &str) {
        let db = self.pool.get().await.expect("a fixture connection");
        db.execute(
            "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\") VALUES ($1, $2::text::jsonb)",
            &[&name, &value],
        )
        .await
        .expect("the config row commits");
    }

    async fn close(self) {
        self.pool.close();
        self.fixture
            .close_async()
            .await
            .expect("the fixture database drops");
    }
}

#[tokio::test]
async fn the_store_reads_the_config_rows_it_serves() {
    let harness = StoreHarness::start().await;
    harness
        .seed_config("general_settings", r#"{"store_model_in_db": true}"#)
        .await;
    harness
        .seed_config("litellm_settings", r#"{"num_retries": 3}"#)
        .await;
    let operations = harness.operations();
    let store = TableConfigParamStore {
        operations: &operations,
    };
    let found = store
        .find_param("general_settings")
        .await
        .expect("the config read runs")
        .expect("the config row exists");
    assert_eq!(found.param_name, "general_settings");
    assert_eq!(found.param_value, json(r#"{"store_model_in_db": true}"#));
    assert!(
        store
            .find_param("absent")
            .await
            .expect("the config miss runs")
            .is_none()
    );
    let mut names: Vec<String> = store
        .find_params(&[
            String::from("general_settings"),
            String::from("litellm_settings"),
        ])
        .await
        .expect("the named read runs")
        .into_iter()
        .map(|row| row.param_name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            String::from("general_settings"),
            String::from("litellm_settings")
        ]
    );
    harness.close().await;
}

#[tokio::test]
async fn the_store_reads_the_user_row() {
    let harness = StoreHarness::start().await;
    let db = harness.pool.get().await.expect("a fixture connection");
    db.execute(
        "INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"spend\") VALUES ('u-1', 'member', 1.5)",
        &[],
    )
    .await
    .expect("the user row commits");
    let operations = harness.operations();
    let store = TableConfigParamStore {
        operations: &operations,
    };
    let found = store
        .user_row("u-1")
        .await
        .expect("the user read runs")
        .expect("the user row exists");
    assert_eq!(found.user_id, "u-1");
    assert_eq!(found.user_alias.as_deref(), Some("member"));
    assert!(
        store
            .user_row("absent")
            .await
            .expect("the user miss runs")
            .is_none()
    );
    harness.close().await;
}
