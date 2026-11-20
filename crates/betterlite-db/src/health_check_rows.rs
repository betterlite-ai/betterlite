use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio_postgres::types::ToSql;

use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;
use row::{decode, decode_all};

const MAX_ERROR_CHARS: usize = 500;

// health_check_latest.py's LATEST_HEALTH_CHECKS_SQL, with details read as text so the column's JSON-string form decodes as _decode_json_text reads it
const LATEST_SQL: &str = r#"SELECT DISTINCT ON ("model_id", "model_name")
       "health_check_id", "model_name", "model_id", "status",
       "healthy_count", "unhealthy_count", "error_message",
       "response_time_ms", "details"::text AS details, "checked_by",
       "checked_at", "created_at", "updated_at"
FROM "LiteLLM_HealthCheckTable"
ORDER BY "model_id" ASC, "model_name" ASC, "checked_at" DESC"#;

// the same statement bounded to the named models, so a paged model list reads one row per pair and nothing else
const LATEST_FOR_MODELS_SQL: &str = r#"SELECT DISTINCT ON ("model_id", "model_name")
       "health_check_id", "model_name", "model_id", "status",
       "healthy_count", "unhealthy_count", "error_message",
       "response_time_ms", "details"::text AS details, "checked_by",
       "checked_at", "created_at", "updated_at"
FROM "LiteLLM_HealthCheckTable"
WHERE "model_name" = ANY($1)
ORDER BY "model_id" ASC, "model_name" ASC, "checked_at" DESC"#;

// get_health_check_history's find_many: equality filters are appended, then newest first with LIMIT and OFFSET
const HISTORY_SQL: &str = r#"SELECT "health_check_id", "model_name", "model_id", "status",
       "healthy_count", "unhealthy_count", "error_message",
       "response_time_ms", "details"::text AS details, "checked_by",
       "checked_at", "created_at", "updated_at"
FROM "LiteLLM_HealthCheckTable""#;

// Prisma fills updated_at with no database default, so the insert stamps it; checked_at and created_at keep their CURRENT_TIMESTAMP defaults
const INSERT_SQL: &str = r#"INSERT INTO "LiteLLM_HealthCheckTable"
    ("health_check_id", "model_name", "model_id", "status", "healthy_count", "unhealthy_count",
     "error_message", "response_time_ms", "checked_by", "updated_at")
VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, CURRENT_TIMESTAMP)
RETURNING "health_check_id", "model_name", "model_id", "status",
       "healthy_count", "unhealthy_count", "error_message",
       "response_time_ms", "details"::text AS details, "checked_by",
       "checked_at", "created_at", "updated_at""#;

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct HealthCheckRow {
    pub health_check_id: String,
    pub model_name: String,
    pub model_id: Option<String>,
    pub status: String,
    pub healthy_count: i32,
    pub unhealthy_count: i32,
    pub error_message: Option<String>,
    pub response_time_ms: Option<f64>,
    pub details: Option<Json>,
    pub checked_by: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// save_health_check_result's arguments: details has no caller at the pin, so it is not a field.
#[derive(Clone, Debug, PartialEq)]
pub struct NewHealthCheck {
    pub model_name: String,
    pub model_id: Option<String>,
    pub status: String,
    pub healthy_count: i32,
    pub unhealthy_count: i32,
    pub error_message: Option<String>,
    pub response_time_ms: Option<f64>,
    pub checked_by: Option<String>,
}

#[derive(Debug, Error)]
pub enum HealthCheckRowsError {
    #[error(transparent)]
    Store(#[from] DbError),
    #[error("the health check table query failed")]
    Query {
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the {column} column could not be decoded")]
    Column {
        column: &'static str,
        #[source]
        source: tokio_postgres::Error,
    },
    #[error("the details column holds text that is not JSON")]
    Details {
        #[source]
        source: serde_json::Error,
    },
}

// the id of a new row; the store takes it from the caller so a test reads its own rows back
pub type HealthCheckIdSource = Arc<dyn Fn() -> String + Send + Sync>;

pub struct HealthCheckRows {
    pub store: PostgresStore,
    pub new_id: HealthCheckIdSource,
}

impl HealthCheckRows {
    // query_latest_health_checks: the erroring form the background save reads
    pub async fn query_latest(&self) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
        match self.store.session().await {
            Ok(client) => decode_all(&client, LATEST_SQL, &[]).await,
            Err(error) => Err(HealthCheckRowsError::from(error)),
        }
    }

    // fetch_latest_health_checks: a driver error is logged and reads as no rows
    pub async fn fetch_latest(&self) -> Vec<HealthCheckRow> {
        match self.query_latest().await {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!("Error getting all latest health checks: {error}");
                Vec::new()
            }
        }
    }

    // fetch_latest_health_checks_for_models: an empty name list reads nothing at all
    pub async fn fetch_latest_for_models(&self, model_names: &[String]) -> Vec<HealthCheckRow> {
        if model_names.is_empty() {
            return Vec::new();
        }
        let rows = self.latest_for_models(model_names).await;
        if let Err(error) = &rows {
            tracing::error!("Error getting latest health checks for models: {error}");
        }
        rows.unwrap_or_default()
    }

    async fn latest_for_models(
        &self,
        model_names: &[String],
    ) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
        let client = self.store.session().await?;
        let names: Vec<&str> = model_names.iter().map(String::as_str).collect();
        decode_all(&client, LATEST_FOR_MODELS_SQL, &[&names]).await
    }

    // get_health_check_history: falsy filters are dropped, the rest is newest first within LIMIT and OFFSET
    pub async fn history(
        &self,
        model_name: Option<&str>,
        status_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Vec<HealthCheckRow> {
        match self
            .history_rows(model_name, status_filter, limit, offset)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!("Error getting health check history: {error}");
                Vec::new()
            }
        }
    }

    async fn history_rows(
        &self,
        model_name: Option<&str>,
        status_filter: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<HealthCheckRow>, HealthCheckRowsError> {
        let model_name = model_name.filter(|name| !name.is_empty());
        let status_filter = status_filter.filter(|status| !status.is_empty());
        let mut parameters: Vec<&(dyn ToSql + Sync)> = Vec::new();
        let mut conditions: Vec<String> = Vec::new();
        if model_name.is_some() {
            parameters.push(&model_name);
            let index = parameters.len();
            conditions.push(format!("\"model_name\" = ${index}"));
        }
        if status_filter.is_some() {
            parameters.push(&status_filter);
            let index = parameters.len();
            conditions.push(format!("\"status\" = ${index}"));
        }
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        parameters.push(&limit);
        let limit_index = parameters.len();
        parameters.push(&offset);
        let offset_index = parameters.len();
        let statement = format!(
            "{HISTORY_SQL}{filter} ORDER BY \"checked_at\" DESC LIMIT ${limit_index} OFFSET ${offset_index}"
        );
        let client = self.store.session().await?;
        decode_all(&client, &statement, &parameters).await
    }

    // save_health_check_result: the row it inserted, or None with the error logged
    pub async fn save_result(&self, new: NewHealthCheck) -> Option<HealthCheckRow> {
        match self.insert(&new).await {
            Ok(row) => row,
            Err(error) => {
                tracing::error!(
                    "Error saving health check result for model {}: {error}",
                    new.model_name
                );
                None
            }
        }
    }

    async fn insert(
        &self,
        new: &NewHealthCheck,
    ) -> Result<Option<HealthCheckRow>, HealthCheckRowsError> {
        let client = self.store.session().await?;
        let health_check_id = (self.new_id)();
        let model_id = new.model_id.as_deref().filter(|id| !id.is_empty());
        let checked_by = new.checked_by.as_deref().filter(|user| !user.is_empty());
        // error_message is kept only when it has text, cut to 500 characters as str(value)[:500] does
        let error_message: Option<String> = new
            .error_message
            .as_deref()
            .filter(|message| !message.is_empty())
            .map(|message| message.chars().take(MAX_ERROR_CHARS).collect());
        // _validate_response_time: a non-finite time is dropped without a warning
        let response_time_ms = new.response_time_ms.filter(|value| value.is_finite());
        let row = client
            .query_opt(
                INSERT_SQL,
                &[
                    &health_check_id,
                    &new.model_name,
                    &model_id,
                    &new.status,
                    &new.healthy_count,
                    &new.unhealthy_count,
                    &error_message,
                    &response_time_ms,
                    &checked_by,
                ],
            )
            .await
            .map_err(|source| HealthCheckRowsError::Query { source })?;
        row.as_ref().map(decode).transpose()
    }
}

mod row;

mod tests;
