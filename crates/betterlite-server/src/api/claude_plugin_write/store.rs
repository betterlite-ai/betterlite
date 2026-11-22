use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::api::claude_plugin_write::StoredPlugin;
use betterlite_db::auxiliary_repository::bindings::ClaudeCodePluginTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::{backend, is_unique_violation};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use chrono::NaiveDateTime;
use thiserror::Error;
use tokio_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

const FIND_SQL: &str = "SELECT id, name, version, description, enabled \
    FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE name = $1";

const CREATE_SQL: &str = "INSERT INTO \"LiteLLM_ClaudeCodePluginTable\" \
    (id, name, version, description, manifest_json, files_json, enabled, created_at, updated_at, created_by) \
    VALUES ($1, $2, $3, $4, $5, '{}', $6, $7, $8, $9) \
    RETURNING id, name, version, description, enabled";

// LiteLLM's update rewrites files_json to '{}' and keeps enabled as the stored row has it
const UPDATE_SQL: &str = "UPDATE \"LiteLLM_ClaudeCodePluginTable\" \
    SET version = $2, description = $3, manifest_json = $4, files_json = '{}', updated_at = $5 \
    WHERE name = $1 RETURNING id, name, version, description, enabled";

const SET_ENABLED_SQL: &str =
    "UPDATE \"LiteLLM_ClaudeCodePluginTable\" SET enabled = $2, updated_at = $3 WHERE name = $1";

const DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE name = $1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewPlugin {
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) manifest_json: String,
    pub(crate) enabled: bool,
    pub(crate) created_at: NaiveDateTime,
    pub(crate) updated_at: NaiveDateTime,
    pub(crate) created_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginChanges {
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) manifest_json: String,
    pub(crate) updated_at: NaiveDateTime,
}

#[derive(Debug, Error)]
pub(crate) enum PluginCreateError {
    #[error("a plugin with this name already exists")]
    NameTaken,
    #[error(transparent)]
    Table(#[from] TableError),
}

#[async_trait]
pub(crate) trait ClaudePluginStore: Send + Sync {
    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPlugin>, TableError>;

    async fn create(&self, write: NewPlugin) -> Result<StoredPlugin, PluginCreateError>;

    async fn update(&self, write: PluginChanges) -> Result<Option<StoredPlugin>, TableError>;

    async fn set_enabled(
        &self,
        plugin_name: &str,
        enabled: bool,
        updated_at: NaiveDateTime,
    ) -> Result<(), TableError>;

    async fn delete(&self, plugin_name: &str) -> Result<(), TableError>;
}

pub(crate) struct PostgresPluginStore {
    store: Arc<PostgresStore>,
    availability: Availability,
}

impl PostgresPluginStore {
    #[must_use]
    pub(crate) fn new(store: Arc<PostgresStore>) -> Self {
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

    async fn read_session(&self) -> Result<Client, TableError> {
        let access = self.access()?;
        let session = access.read.session().await;
        session.map_err(TableError::Database)
    }

    async fn write_session(&self) -> Result<Client, TableError> {
        let access = self.access()?;
        let session = access.write.session().await;
        session.map_err(TableError::Database)
    }
}

#[async_trait]
impl ClaudePluginStore for PostgresPluginStore {
    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPlugin>, TableError> {
        let session = self.read_session().await?;
        let found = session
            .query_opt(FIND_SQL, &[&plugin_name])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        found.as_ref().map(decode_row).transpose()
    }

    async fn create(&self, write: NewPlugin) -> Result<StoredPlugin, PluginCreateError> {
        let session = self.write_session().await?;
        let id = Uuid::new_v4().to_string();
        let parameters: [&(dyn ToSql + Sync); 9] = [
            &id,
            &write.name,
            &write.version,
            &write.description,
            &write.manifest_json,
            &write.enabled,
            &write.created_at,
            &write.updated_at,
            &write.created_by,
        ];
        let inserted = session.query_opt(CREATE_SQL, &parameters).await;
        let row = match inserted {
            Ok(Some(row)) => row,
            Ok(None) => return Err(PluginCreateError::Table(TableError::MissingRow)),
            Err(source) => {
                let error = backend(source);
                if is_unique_violation(&error) {
                    return Err(PluginCreateError::NameTaken);
                }
                return Err(PluginCreateError::Table(TableError::Database(error)));
            }
        };
        decode_row(&row).map_err(PluginCreateError::Table)
    }

    async fn update(&self, write: PluginChanges) -> Result<Option<StoredPlugin>, TableError> {
        let session = self.write_session().await?;
        let parameters: [&(dyn ToSql + Sync); 5] = [
            &write.name,
            &write.version,
            &write.description,
            &write.manifest_json,
            &write.updated_at,
        ];
        let found = session
            .query_opt(UPDATE_SQL, &parameters)
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        found.as_ref().map(decode_row).transpose()
    }

    async fn set_enabled(
        &self,
        plugin_name: &str,
        enabled: bool,
        updated_at: NaiveDateTime,
    ) -> Result<(), TableError> {
        let session = self.write_session().await?;
        let parameters: [&(dyn ToSql + Sync); 3] = [&plugin_name, &enabled, &updated_at];
        session
            .execute(SET_ENABLED_SQL, &parameters)
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(())
    }

    async fn delete(&self, plugin_name: &str) -> Result<(), TableError> {
        let session = self.write_session().await?;
        session
            .execute(DELETE_SQL, &[&plugin_name])
            .await
            .map_err(|source| TableError::Database(backend(source)))?;
        Ok(())
    }
}

fn decode_row(row: &Row) -> Result<StoredPlugin, TableError> {
    let id: String = row
        .try_get("id")
        .map_err(|source| TableError::Database(backend(source)))?;
    let name: String = row
        .try_get("name")
        .map_err(|source| TableError::Database(backend(source)))?;
    let version: Option<String> = row
        .try_get("version")
        .map_err(|source| TableError::Database(backend(source)))?;
    let description: Option<String> = row
        .try_get("description")
        .map_err(|source| TableError::Database(backend(source)))?;
    let enabled: bool = row
        .try_get("enabled")
        .map_err(|source| TableError::Database(backend(source)))?;
    Ok(StoredPlugin {
        id,
        name,
        version,
        description,
        enabled,
    })
}
