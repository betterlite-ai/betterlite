use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auxiliary_routes::INFO_ROUTES;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use tokio_postgres::Row;
use tokio_postgres::types::{Json, ToSql};

use super::model::{CacheActivityErrorBucket, CacheActivityGroup, CacheActivityRows};
use super::sql::{ERROR_BREAKDOWN_SQL, GROUPS_SQL, KEY_ALIAS_OPTIONS_SQL, MODEL_OPTIONS_SQL};

type Parameter<'parameter> = &'parameter (dyn ToSql + Sync);

#[async_trait]
pub trait CacheActivityStore: Send + Sync {
    async fn read(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        key_aliases: &[String],
        models: &[String],
    ) -> Result<CacheActivityRows, DbError>;
}

pub struct DatabaseCacheActivityStore {
    pub store: Arc<PostgresStore>,
}

#[async_trait]
impl CacheActivityStore for DatabaseCacheActivityStore {
    async fn read(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        key_aliases: &[String],
        models: &[String],
    ) -> Result<CacheActivityRows, DbError> {
        let client = self.store.session().await?;
        let key_aliases_json = Json(key_aliases);
        let models_json = Json(models);
        let info_routes_json = Json(INFO_ROUTES);
        let filtered: [Parameter<'_>; 5] = [
            &start,
            &end,
            &key_aliases_json,
            &models_json,
            &info_routes_json,
        ];
        let options: [Parameter<'_>; 3] = [&start, &end, &info_routes_json];
        let (group_rows, error_rows, key_alias_rows, model_rows) = tokio::try_join!(
            client.query(GROUPS_SQL, &filtered),
            client.query(ERROR_BREAKDOWN_SQL, &filtered),
            client.query(KEY_ALIAS_OPTIONS_SQL, &options),
            client.query(MODEL_OPTIONS_SQL, &options),
        )
        .map_err(backend)?;
        decode(&group_rows, &error_rows, &key_alias_rows, &model_rows)
    }
}

fn decode(
    group_rows: &[Row],
    error_rows: &[Row],
    key_alias_rows: &[Row],
    model_rows: &[Row],
) -> Result<CacheActivityRows, DbError> {
    Ok(CacheActivityRows {
        groups: group_rows
            .iter()
            .map(CacheActivityGroup::try_from)
            .collect::<Result<Vec<_>, DbError>>()?,
        error_breakdown: error_rows
            .iter()
            .map(CacheActivityErrorBucket::try_from)
            .collect::<Result<Vec<_>, DbError>>()?,
        key_aliases: key_alias_rows
            .iter()
            .map(KeyAliasRow::try_from)
            .map(|decoded| decoded.map(|row| row.key_alias))
            .collect::<Result<Vec<String>, DbError>>()?,
        models: model_rows
            .iter()
            .map(ModelRow::try_from)
            .map(|decoded| decoded.map(|row| row.model))
            .collect::<Result<Vec<String>, DbError>>()?,
    })
}

impl TryFrom<&Row> for CacheActivityGroup {
    type Error = DbError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            call_type: row.try_get("call_type").map_err(backend)?,
            api_requests: i64::from(row.try_get::<_, i32>("api_requests").map_err(backend)?),
            cache_hits: i64::from(row.try_get::<_, i32>("cache_hits").map_err(backend)?),
            failed_requests: i64::from(row.try_get::<_, i32>("failed_requests").map_err(backend)?),
            cached_completion_tokens: i64::from(
                row.try_get::<_, i32>("cached_completion_tokens")
                    .map_err(backend)?,
            ),
            generated_completion_tokens: i64::from(
                row.try_get::<_, i32>("generated_completion_tokens")
                    .map_err(backend)?,
            ),
        })
    }
}

impl TryFrom<&Row> for CacheActivityErrorBucket {
    type Error = DbError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        let call_type: String = row.try_get("call_type").map_err(backend)?;
        let error_code: String = row.try_get("error_code").map_err(backend)?;
        let error_class: String = row.try_get("error_class").map_err(backend)?;
        let failures: i32 = row.try_get("count").map_err(backend)?;
        Ok(Self {
            call_type,
            error_code,
            error_class,
            count: i64::from(failures),
        })
    }
}

struct KeyAliasRow {
    key_alias: String,
}

impl TryFrom<&Row> for KeyAliasRow {
    type Error = DbError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            key_alias: row.try_get("key_alias").map_err(backend)?,
        })
    }
}

struct ModelRow {
    model: String,
}

impl TryFrom<&Row> for ModelRow {
    type Error = DbError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            model: row.try_get("model").map_err(backend)?,
        })
    }
}
