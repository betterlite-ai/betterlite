use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::ModelTable;
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableAccess, TableError,
};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError};
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_models::decisions::Json;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_postgres::types::ToSql;

use super::alias::{AliasMap, AliasRow, AliasRows};
use super::deployments::{DeploymentRow, DeploymentRows};
use super::error::CleanupError;
use super::remove::TeamModelRows;

const ALIAS_ROWS_SQL: &str = r#"SELECT alias_row.id::BIGINT, alias_row.aliases::TEXT, team.team_id
    FROM "LiteLLM_ModelTable" alias_row
    LEFT JOIN "LiteLLM_TeamTable" team ON team.model_id = alias_row.id
    ORDER BY alias_row.id"#;
const UPDATE_ALIASES_SQL: &str = r#"UPDATE "LiteLLM_ModelTable"
    SET aliases = $2::TEXT::JSONB, updated_at = CURRENT_TIMESTAMP
    WHERE id = $1"#;
// starts_with, not LIKE: an underscore in the prefix is a literal, as in Prisma's startswith.
const DEPLOYMENT_ROWS_SQL: &str = r#"SELECT model_name, model_info::TEXT
    FROM "LiteLLM_ProxyModelTable"
    WHERE starts_with(model_name, $1)
    ORDER BY model_id"#;
const TEAM_MODELS_SQL: &str =
    r#"SELECT COALESCE(models, '{}') FROM "LiteLLM_TeamTable" WHERE team_id = $1"#;
const UPDATE_TEAM_MODELS_SQL: &str = r#"UPDATE "LiteLLM_TeamTable" AS team
    SET models = $2, updated_at = CURRENT_TIMESTAMP
    WHERE team_id = $1
    RETURNING row_to_json(team)::TEXT"#;

// The three tables the cleanup reads and writes, on the writer like LiteLLM's single Prisma client:
// the deployment row is already deleted, so a lagging replica must not decide the sibling check.
pub struct PostgresTeamModels<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl<'db> PostgresTeamModels<'db> {
    fn access(&self) -> Result<TableAccess<'db>, DbError> {
        PrismaTableRepository::new(self.client, redis_config_change_publisher(None), ModelTable)
            .with_writer()
            .access()
    }
}

#[async_trait]
impl AliasRows for PostgresTeamModels<'_> {
    async fn alias_rows(&self) -> Result<Vec<AliasRow>, CleanupError> {
        let access = self.access()?;
        let client = access.read.session().await?;
        let rows = client.query(ALIAS_ROWS_SQL, &[]).await.map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            let id: i64 = row.try_get(0).map_err(backend)?;
            let aliases: Option<String> = row.try_get(1).map_err(backend)?;
            let team_id: Option<String> = row.try_get(2).map_err(backend)?;
            found.push(AliasRow {
                id,
                team_id,
                aliases: alias_map(id, aliases)?,
            });
        }
        Ok(found)
    }

    async fn save_aliases(&self, id: i64, aliases: &AliasMap) -> Result<(), CleanupError> {
        let text = serde_json::to_string(aliases).map_err(TableError::Row)?;
        let access = self.access()?;
        let client = access.write.session().await?;
        let parameters: [&(dyn ToSql + Sync); 2] = [&id, &text];
        client
            .execute(UPDATE_ALIASES_SQL, &parameters)
            .await
            .map_err(backend)?;
        Ok(())
    }
}

#[async_trait]
impl DeploymentRows for PostgresTeamModels<'_> {
    async fn deployments_with_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<DeploymentRow>, CleanupError> {
        let access = self.access()?;
        let client = access.read.session().await?;
        let parameters: [&(dyn ToSql + Sync); 1] = [&prefix];
        let rows = client
            .query(DEPLOYMENT_ROWS_SQL, &parameters)
            .await
            .map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            let model_name: String = row.try_get(0).map_err(backend)?;
            let model_info: Option<String> = row.try_get(1).map_err(backend)?;
            found.push(DeploymentRow {
                model_name,
                model_info: parse_json(model_info)?,
            });
        }
        Ok(found)
    }
}

#[async_trait]
impl TeamModelRows for PostgresTeamModels<'_> {
    async fn team_models(&self, team_id: &str) -> Result<Option<Vec<String>>, CleanupError> {
        let access = self.access()?;
        let client = access.read.session().await?;
        let found = client
            .query_opt(TEAM_MODELS_SQL, &[&team_id])
            .await
            .map_err(backend)?;
        let Some(row) = found else {
            return Ok(None);
        };
        let models: Vec<String> = row.try_get(0).map_err(backend)?;
        Ok(Some(models))
    }

    async fn save_team_models(
        &self,
        team_id: &str,
        models: &[String],
    ) -> Result<TeamRecord, CleanupError> {
        let access = self.access()?;
        let client = access.write.session().await?;
        let parameters: [&(dyn ToSql + Sync); 2] = [&team_id, &models];
        let found = client
            .query_opt(UPDATE_TEAM_MODELS_SQL, &parameters)
            .await
            .map_err(backend)?;
        let Some(row) = found else {
            return Err(not_a_team_row());
        };
        let text: String = row.try_get(0).map_err(backend)?;
        let parsed = TeamRecord::parse(text.as_bytes()).map_err(TeamQueryError::from)?;
        match parsed {
            Some(record) => Ok(record),
            None => Err(not_a_team_row()),
        }
    }
}

fn not_a_team_row() -> CleanupError {
    CleanupError::from(TeamQueryError::from(TeamRecordError::NotAnObject))
}

fn alias_map(id: i64, stored: Option<String>) -> Result<Option<AliasMap>, CleanupError> {
    let Some(text) = stored else {
        return Ok(None);
    };
    match parse_json(Some(text))? {
        Json::Null => Ok(None),
        Json::Object(mapping) => Ok(Some(mapping)),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(CleanupError::Aliases { id })
        }
    }
}

fn parse_json(stored: Option<String>) -> Result<Json, CleanupError> {
    let Some(text) = stored else {
        return Ok(Json::Null);
    };
    serde_json::from_str(&text).map_err(|source| CleanupError::from(TableError::Row(source)))
}
