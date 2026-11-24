use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::ClaudeCodePluginTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use chrono::NaiveDateTime;
use serde::Deserialize;
use tokio_postgres::Row;

const ENABLED_PLUGINS_SQL: &str = "SELECT id, name, manifest_json, enabled, created_at, updated_at \
    FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE enabled = TRUE";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PluginRow {
    pub id: String,
    pub name: String,
    pub manifest_json: Option<String>,
    pub enabled: bool,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
}

#[async_trait]
pub trait PluginTable: Send + Sync {
    async fn enabled_plugins(&self) -> Result<Vec<PluginRow>, TableError>;
}

pub struct PostgresPluginTable {
    store: Arc<PostgresStore>,
    availability: Availability,
}

impl PostgresPluginTable {
    #[must_use]
    pub fn new(store: Arc<PostgresStore>) -> Self {
        Self {
            store,
            availability: Availability::default(),
        }
    }

    fn access(&self) -> Result<TableAccess<'_>, TableError> {
        let client = PrismaClient {
            writer: self.store.as_ref(),
            reader: self.store.as_ref(),
            availability: &self.availability,
        };
        PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            ClaudeCodePluginTable,
        )
        .access()
        .map_err(TableError::Database)
    }
}

#[async_trait]
impl PluginTable for PostgresPluginTable {
    async fn enabled_plugins(&self) -> Result<Vec<PluginRow>, TableError> {
        let access = self.access()?;
        let session = access.read.session().await.map_err(TableError::Database)?;
        let rows = session
            .query(ENABLED_PLUGINS_SQL, &[])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        rows.iter().map(decode_row).collect()
    }
}

fn decode_row(row: &Row) -> Result<PluginRow, TableError> {
    let id: String = row
        .try_get("id")
        .map_err(|source| TableError::Database(backend(source)))?;
    let name: String = row
        .try_get("name")
        .map_err(|source| TableError::Database(backend(source)))?;
    let manifest_json: Option<String> = row
        .try_get("manifest_json")
        .map_err(|source| TableError::Database(backend(source)))?;
    let enabled: bool = row
        .try_get("enabled")
        .map_err(|source| TableError::Database(backend(source)))?;
    let created_at: Option<NaiveDateTime> = row
        .try_get("created_at")
        .map_err(|source| TableError::Database(backend(source)))?;
    let updated_at: Option<NaiveDateTime> = row
        .try_get("updated_at")
        .map_err(|source| TableError::Database(backend(source)))?;
    Ok(PluginRow {
        id,
        name,
        manifest_json,
        enabled,
        created_at,
        updated_at,
    })
}
