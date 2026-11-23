use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::api::claude_plugin_policy::{PluginWhere, PluginWhereBranch};
use betterlite_db::auxiliary_repository::bindings::ClaudeCodePluginTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableError,
};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

const LIST_SQL: &str = "SELECT id, name, version, description, manifest_json, enabled, created_at, updated_at, created_by \
    FROM \"LiteLLM_ClaudeCodePluginTable\"";

const LIST_ENABLED_SQL: &str = "SELECT id, name, version, description, manifest_json, enabled, created_at, updated_at, created_by \
    FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE enabled = TRUE";

// SkillVisibility.where(): the enabled branch plus the granted names find_many gets as name in [...]
const LIST_VISIBLE_SQL: &str = "SELECT id, name, version, description, manifest_json, enabled, created_at, updated_at, created_by \
    FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE enabled = TRUE OR name = ANY($1::text[])";

const FIND_SQL: &str = "SELECT id, name, version, description, manifest_json, enabled, created_at, updated_at, created_by \
    FROM \"LiteLLM_ClaudeCodePluginTable\" WHERE name = $1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredPluginRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) manifest_json: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) created_at: Option<DateTime<Utc>>,
    pub(crate) updated_at: Option<DateTime<Utc>>,
    pub(crate) created_by: Option<String>,
}

#[async_trait]
pub(crate) trait ClaudePluginReadStore: Send + Sync {
    async fn list(&self, filter: &PluginWhere) -> Result<Vec<StoredPluginRow>, TableError>;

    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPluginRow>, TableError>;
}

pub(crate) struct PostgresPluginReadStore {
    store: Arc<PostgresStore>,
    availability: Availability,
}

impl PostgresPluginReadStore {
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
}

#[async_trait]
impl ClaudePluginReadStore for PostgresPluginReadStore {
    async fn list(&self, filter: &PluginWhere) -> Result<Vec<StoredPluginRow>, TableError> {
        let session = self.read_session().await?;
        let rows = match filter.any.as_deref() {
            Some(branches) => {
                let names = granted_names(branches);
                let parameters: [&(dyn ToSql + Sync); 1] = [&names];
                session.query(LIST_VISIBLE_SQL, &parameters).await
            }
            None if filter.enabled == Some(true) => session.query(LIST_ENABLED_SQL, &[]).await,
            None => session.query(LIST_SQL, &[]).await,
        }
        .map_err(|source| TableError::Database(backend(source)))?;
        rows.iter().map(decode_row).collect()
    }

    async fn find_by_name(&self, plugin_name: &str) -> Result<Option<StoredPluginRow>, TableError> {
        let session = self.read_session().await?;
        let row = match session.query_opt(FIND_SQL, &[&plugin_name]).await {
            Ok(row) => row,
            Err(source) => return Err(TableError::Database(backend(source))),
        };
        match row.as_ref() {
            Some(row) => decode_row(row).map(Some),
            None => Ok(None),
        }
    }
}

fn granted_names(branches: &[PluginWhereBranch]) -> Vec<String> {
    let mut names = Vec::new();
    for branch in branches {
        if let PluginWhereBranch::Name { name } = branch {
            names.extend(name.names.iter().cloned());
        }
    }
    names
}

fn decode_row(row: &Row) -> Result<StoredPluginRow, TableError> {
    let fail = |source| TableError::Database(backend(source));
    let id: String = row.try_get("id").map_err(fail)?;
    let name: String = row.try_get("name").map_err(fail)?;
    let version: Option<String> = row.try_get("version").map_err(fail)?;
    let description: Option<String> = row.try_get("description").map_err(fail)?;
    let manifest_json: Option<String> = row.try_get("manifest_json").map_err(fail)?;
    let enabled: bool = row.try_get("enabled").map_err(fail)?;
    let created_at: Option<NaiveDateTime> = row.try_get("created_at").map_err(fail)?;
    let updated_at: Option<NaiveDateTime> = row.try_get("updated_at").map_err(fail)?;
    Ok(StoredPluginRow {
        id,
        name,
        version,
        description,
        manifest_json,
        enabled,
        created_at: created_at.map(|value| value.and_utc()),
        updated_at: updated_at.map(|value| value.and_utc()),
        created_by: row.try_get("created_by").map_err(fail)?,
    })
}
