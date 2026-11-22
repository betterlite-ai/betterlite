use std::sync::atomic::Ordering;

use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_models::decisions::Json;
use tokio_postgres::types::ToSql;

use super::errors::{GroupDataError, HttpFailure};
use super::ports::{BudgetInclude, GroupBudgetCreate, GroupBudgetUpdate, GroupDatabase};
use super::types::{DeploymentRow, GroupBudgetRow};

const DEPLOYMENTS_SQL: &str =
    r#"SELECT model_id, model_name, model_info::TEXT FROM "LiteLLM_ProxyModelTable""#;
const DEPLOYMENTS_BY_NAME_SQL: &str = r#"SELECT model_id, model_name, model_info::TEXT
    FROM "LiteLLM_ProxyModelTable" WHERE model_name = ANY($1::text[])"#;
const DEPLOYMENT_SQL: &str = r#"SELECT model_id, model_name, model_info::TEXT
    FROM "LiteLLM_ProxyModelTable" WHERE model_id = $1"#;
const UPDATE_MODEL_INFO_SQL: &str = r#"UPDATE "LiteLLM_ProxyModelTable" AS entry
    SET model_info = $2::TEXT::JSONB, updated_at = CURRENT_TIMESTAMP WHERE entry.model_id = $1"#;
// A missing or dangling budget_id leaves litellm_budget_table null, so find never invents a budget.
const GROUP_BUDGET_ONE_SQL: &str = r#"SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(budget) FROM "LiteLLM_BudgetTable" AS budget WHERE budget."budget_id" = entry."budget_id")))::TEXT FROM "LiteLLM_ModelAccessGroupBudgetTable" AS entry WHERE entry."access_group_name" = $1"#;
const GROUP_BUDGET_MANY_SQL: &str = r#"SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(budget) FROM "LiteLLM_BudgetTable" AS budget WHERE budget."budget_id" = entry."budget_id")))::TEXT FROM "LiteLLM_ModelAccessGroupBudgetTable" AS entry"#;
const GROUP_BUDGET_ONE_PLAIN_SQL: &str = r#"SELECT to_jsonb(entry)::TEXT
    FROM "LiteLLM_ModelAccessGroupBudgetTable" AS entry WHERE entry."access_group_name" = $1"#;
const GROUP_BUDGET_MANY_PLAIN_SQL: &str =
    r#"SELECT to_jsonb(entry)::TEXT FROM "LiteLLM_ModelAccessGroupBudgetTable" AS entry"#;
const GROUP_BUDGET_DELETE_SQL: &str = r#"DELETE FROM "LiteLLM_ModelAccessGroupBudgetTable" AS entry
    WHERE entry."access_group_name" = $1 RETURNING to_jsonb(entry)::TEXT"#;
// create fills the row, update owns every column a rewrite may change; spend and created_by stay.
const GROUP_BUDGET_UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" AS entry
    (access_group_name, budget_id, created_by, updated_by, updated_at)
    VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
    ON CONFLICT (access_group_name) DO UPDATE
    SET budget_id = $5, updated_by = $6, updated_at = CURRENT_TIMESTAMP
    RETURNING (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(budget) FROM "LiteLLM_BudgetTable" AS budget WHERE budget."budget_id" = entry."budget_id")))::TEXT"#;
const GROUP_BUDGET_UPSERT_PLAIN_SQL: &str = r#"INSERT INTO "LiteLLM_ModelAccessGroupBudgetTable" AS entry
    (access_group_name, budget_id, created_by, updated_by, updated_at)
    VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
    ON CONFLICT (access_group_name) DO UPDATE
    SET budget_id = $5, updated_by = $6, updated_at = CURRENT_TIMESTAMP
    RETURNING to_jsonb(entry)::TEXT"#;

pub struct PostgresGroupData<'db> {
    pub client: Option<PrismaClient<'db>>,
}

impl<'db> PostgresGroupData<'db> {
    fn reader(&self) -> Result<&'db PostgresStore, GroupDataError> {
        let client = self.client.ok_or_else(database_missing)?;
        if client
            .availability
            .reader_unavailable
            .load(Ordering::SeqCst)
        {
            return Ok(client.writer);
        }
        Ok(client.reader)
    }

    fn writer(&self) -> Result<&'db PostgresStore, GroupDataError> {
        if let Some(client) = self.client {
            return Ok(client.writer);
        }
        Err(database_missing())
    }
}

// _prisma_client_or_500: a pod with no database answers this module's 500, not a driver error.
fn database_missing() -> GroupDataError {
    GroupDataError::Http(HttpFailure {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: "Database not connected.".to_owned(),
    })
}

fn decode_deployment(row: &tokio_postgres::Row) -> Result<DeploymentRow, GroupDataError> {
    let model_id: String = row.try_get(0).map_err(backend)?;
    let model_name: String = row.try_get(1).map_err(backend)?;
    let stored: Option<String> = row.try_get(2).map_err(backend)?;
    let model_info = stored
        .map(|text| serde_json::from_str::<Json>(&text))
        .transpose()
        .map_err(GroupDataError::Row)?;
    Ok(DeploymentRow {
        model_id,
        model_name,
        model_info: model_info.filter(|value| !matches!(value, Json::Null)),
    })
}

fn decode_group_budget(row: &tokio_postgres::Row) -> Result<GroupBudgetRow, GroupDataError> {
    serde_json::from_str(&row.try_get::<_, String>(0).map_err(backend)?)
        .map_err(GroupDataError::Row)
}

// The single-row reads and the delete share one query shape: one optional row, then its decode.
async fn query_optional(
    store: &PostgresStore,
    sql: &str,
    parameter: &(dyn ToSql + Sync),
) -> Result<Option<tokio_postgres::Row>, DbError> {
    let client = store.session().await?;
    let parameters: [&(dyn ToSql + Sync); 1] = [parameter];
    client.query_opt(sql, &parameters).await.map_err(backend)
}

fn decode_optional<T>(
    row: Option<&tokio_postgres::Row>,
    decode: fn(&tokio_postgres::Row) -> Result<T, GroupDataError>,
) -> Result<Option<T>, GroupDataError> {
    match row {
        Some(row) => Ok(Some(decode(row)?)),
        None => Ok(None),
    }
}

impl GroupDatabase for PostgresGroupData<'_> {
    async fn deployment_find_many(
        &self,
        model_names: Option<&[String]>,
    ) -> Result<Vec<DeploymentRow>, GroupDataError> {
        let store = self.reader()?;
        let client = store.session().await?;
        let rows = match model_names {
            Some(names) => {
                let parameters: [&(dyn ToSql + Sync); 1] = [&names];
                client
                    .query(DEPLOYMENTS_BY_NAME_SQL, &parameters)
                    .await
                    .map_err(backend)?
            }
            None => client.query(DEPLOYMENTS_SQL, &[]).await.map_err(backend)?,
        };
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            found.push(decode_deployment(row)?);
        }
        Ok(found)
    }

    async fn deployment_find_unique(
        &self,
        model_id: &str,
    ) -> Result<Option<DeploymentRow>, GroupDataError> {
        let row = query_optional(self.reader()?, DEPLOYMENT_SQL, model_id).await?;
        decode_optional(row.as_ref(), decode_deployment)
    }

    async fn update_model_info(
        &self,
        model_id: &str,
        model_info: &str,
    ) -> Result<(), GroupDataError> {
        let store = self.writer()?;
        let client = store.session().await?;
        let parameters: [&(dyn ToSql + Sync); 2] = [&model_id, &model_info];
        client
            .execute(UPDATE_MODEL_INFO_SQL, &parameters)
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn budget_find_unique(
        &self,
        access_group: &str,
        include: BudgetInclude,
    ) -> Result<Option<GroupBudgetRow>, GroupDataError> {
        let sql = if include.litellm_budget_table {
            GROUP_BUDGET_ONE_SQL
        } else {
            GROUP_BUDGET_ONE_PLAIN_SQL
        };
        let row = query_optional(self.reader()?, sql, access_group).await?;
        decode_optional(row.as_ref(), decode_group_budget)
    }

    async fn budget_find_many(
        &self,
        include: BudgetInclude,
    ) -> Result<Vec<GroupBudgetRow>, GroupDataError> {
        let store = self.reader()?;
        let client = store.session().await?;
        let sql = if include.litellm_budget_table {
            GROUP_BUDGET_MANY_SQL
        } else {
            GROUP_BUDGET_MANY_PLAIN_SQL
        };
        let rows = client.query(sql, &[]).await.map_err(backend)?;
        let mut found = Vec::with_capacity(rows.len());
        for row in &rows {
            found.push(decode_group_budget(row)?);
        }
        Ok(found)
    }

    async fn budget_upsert(
        &self,
        create: &GroupBudgetCreate,
        update: &GroupBudgetUpdate,
        include: BudgetInclude,
    ) -> Result<GroupBudgetRow, GroupDataError> {
        let store = self.writer()?;
        let client = store.session().await?;
        let sql = if include.litellm_budget_table {
            GROUP_BUDGET_UPSERT_SQL
        } else {
            GROUP_BUDGET_UPSERT_PLAIN_SQL
        };
        let parameters: [&(dyn ToSql + Sync); 6] = [
            &create.access_group_name,
            &create.budget_id,
            &create.created_by,
            &create.updated_by,
            &update.budget_id,
            &update.updated_by,
        ];
        let row = client.query_one(sql, &parameters).await.map_err(backend)?;
        decode_group_budget(&row)
    }

    async fn budget_delete(
        &self,
        access_group: &str,
    ) -> Result<Option<GroupBudgetRow>, GroupDataError> {
        let row = query_optional(self.writer()?, GROUP_BUDGET_DELETE_SQL, access_group).await?;
        decode_optional(row.as_ref(), decode_group_budget)
    }
}
