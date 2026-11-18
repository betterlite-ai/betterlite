#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{NaiveDate, NaiveDateTime};
use clap::Parser;
use deadpool_postgres::{GenericClient as _, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use tokio::task::yield_now;
use tokio_postgres::NoTls;

use crate::auxiliary_repository::repository::PrismaClient;
use crate::budget_permission_rows::JsonData;
use crate::database_replica::availability::Availability;
use crate::migration_ledger::baseline::{PACKAGED_CATALOGS, catalog_statements};
use crate::postgres_store::session::PostgresStore;
use crate::reload_schedule::RELOAD_CONFIG_PARAM_NAME;
use crate::reload_schedule::evictor::{ConfigParamEvictor, EvictError, EvictFuture};

const SEED_SQL: &str = r#"INSERT INTO "LiteLLM_Config" ("param_name", "param_value", "last_run_at", "reload_revision")
VALUES ($1, $2::text::jsonb, $3, $4)"#;

const READ_ROW_SQL: &str =
    r#"SELECT row_to_json(t)::text FROM "LiteLLM_Config" AS t WHERE t."param_name" = $1"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

pub(crate) struct Database {
    fixture: PostgresFixture,
    pub(crate) store: PostgresStore,
    availability: Availability,
}

impl Database {
    pub(crate) async fn provision() -> Self {
        let url = Environment::parse_from(["reload-schedule-tests"])
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
        let pool = Pool::builder(Manager::new(config, NoTls))
            .max_size(4)
            .build()
            .unwrap();
        Self {
            fixture,
            store: PostgresStore { pool: Some(pool) },
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
        // the closed pool's drivers need a turn to close their sockets before DROP DATABASE
        yield_now().await;
        self.fixture.close_async().await.unwrap();
    }
}

#[derive(Clone, Default)]
pub(crate) struct RecordingEvictor {
    names: Arc<Mutex<Vec<String>>>,
}

impl RecordingEvictor {
    pub(crate) fn names(&self) -> Vec<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ConfigParamEvictor for RecordingEvictor {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call> {
        let names = Arc::clone(&self.names);
        let name = param_name.to_owned();
        Box::pin(async move {
            names
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(name);
            Ok(())
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct FailingEvictor;

impl ConfigParamEvictor for FailingEvictor {
    fn evict_param<'call>(&'call self, param_name: &'call str) -> EvictFuture<'call> {
        let refusal = EvictError {
            param_name: param_name.to_owned(),
            source: Box::new(std::io::Error::other("the cache refused the eviction")),
        };
        Box::pin(std::future::ready(Err(refusal)))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoredRow {
    pub(crate) param_value: JsonData,
    pub(crate) last_run_at: Option<NaiveDateTime>,
    pub(crate) reload_revision: i64,
}

pub(crate) async fn stored_row(database: &Database) -> Option<StoredRow> {
    let session = database.store.session().await.unwrap();
    let row = session
        .query_opt(READ_ROW_SQL, &[&RELOAD_CONFIG_PARAM_NAME])
        .await
        .unwrap()?;
    let text: String = row.get(0);
    Some(serde_json::from_str(&text).unwrap())
}

pub(crate) async fn seed_row(
    database: &Database,
    param_value: Option<JsonData>,
    last_run_at: Option<NaiveDateTime>,
    reload_revision: i64,
) {
    let session = database.store.session().await.unwrap();
    let encoded = param_value.map(|value| serde_json::to_string(&value).unwrap());
    session
        .execute(
            SEED_SQL,
            &[
                &RELOAD_CONFIG_PARAM_NAME,
                &encoded,
                &last_run_at,
                &reload_revision,
            ],
        )
        .await
        .unwrap();
}

pub(crate) fn instant() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 3, 4)
        .unwrap()
        .and_hms_opt(5, 6, 7)
        .unwrap()
}

pub(crate) fn serialized_interval(hours: u32) -> JsonData {
    JsonData::String(format!("{{\"interval_hours\": {hours}}}"))
}
