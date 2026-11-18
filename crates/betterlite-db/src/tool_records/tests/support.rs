use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Client, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::yield_now;
use tokio_postgres::NoTls;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Db {
    fixture: PostgresFixture,
    store: PostgresStore,
}

impl Db {
    pub(crate) async fn start() -> Self {
        let url = Environment::parse_from(["tool-records"])
            .url
            .map(SecretString::from);
        let fixture = PostgresFixture::provision_with_statements(
            url.as_ref(),
            catalog_statements(&PACKAGED_CATALOGS).unwrap(),
        )
        .resolve()
        .await
        .unwrap();
        let config = fixture.options.url.expose_secret().parse().unwrap();
        let manager = Manager::new(config, NoTls);
        let pool = Pool::builder(manager).max_size(2).build().unwrap();
        let store = PostgresStore { pool: Some(pool) };
        Self { fixture, store }
    }

    pub(crate) async fn client(&self) -> Client {
        self.store.session().await.unwrap()
    }

    pub(crate) async fn batch(&self, sql: &str) {
        let client = self.client().await;
        client.batch_execute(sql).await.unwrap();
    }

    pub(crate) async fn close(self) {
        self.store.pool.unwrap().close();
        // The closed pool's drivers need a turn to close their sockets before DROP DATABASE.
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

impl CapturedLogs {
    pub(crate) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for CapturedLogs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'writer> MakeWriter<'writer> for CapturedLogs {
    type Writer = Self;

    fn make_writer(&'writer self) -> Self::Writer {
        self.clone()
    }
}

pub(crate) fn captured_logs() -> (CapturedLogs, DefaultGuard) {
    let logs = CapturedLogs::default();
    let guard = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .set_default();
    (logs, guard)
}
