use async_trait::async_trait;
use betterlite_db::database_views::definitions::{
    LAST_30D_KEYS_BY_SPEND, LAST_30D_MODELS_BY_SPEND, MONTHLY_GLOBAL_SPEND,
    MONTHLY_GLOBAL_SPEND_PER_KEY, MONTHLY_USER_KEY_SPEND,
};
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::rows::{
    DailySpend, KeyDailySpend, KeySpend, ModelSpend, TeamSpend, TotalSpendRow, UserKeyDailySpend,
    UserModelSpend,
};

mod decoders;

const USER_KEYS_SQL: &str = "WITH top_api_keys AS (SELECT api_key, SUM(spend) AS total_spend FROM \"LiteLLM_SpendLogs\" WHERE \"user\" = $1 GROUP BY api_key ORDER BY total_spend DESC LIMIT $2) SELECT t.api_key, t.total_spend::text AS spend_text, v.key_alias, v.key_name FROM top_api_keys t LEFT JOIN \"LiteLLM_VerificationToken\" v ON t.api_key = v.token ORDER BY t.total_spend DESC";
const USER_MODELS_SQL: &str = "SELECT model, SUM(spend)::text AS spend_text, SUM(total_tokens)::bigint AS total_tokens FROM \"LiteLLM_SpendLogs\" WHERE \"user\" = $1 GROUP BY model ORDER BY SUM(spend) DESC LIMIT $2";
const TEAM_DAILY_SQL: &str = "SELECT t.team_alias AS team_alias, DATE(s.\"startTime\") AS spend_date, SUM(s.spend) AS total_spend FROM \"LiteLLM_SpendLogs\" s LEFT JOIN \"LiteLLM_TeamTable\" t ON s.team_id = t.team_id WHERE s.\"startTime\" >= CURRENT_DATE - INTERVAL '30 days' GROUP BY t.team_alias, DATE(s.\"startTime\") ORDER BY spend_date";

#[derive(Debug, thiserror::Error)]
pub enum TotalsStoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the database returned an unreadable value for the {column} column")]
    InvalidAmount { column: &'static str },
    #[error("the database returned an unreadable value for the {column} column")]
    UnreadableAmount {
        column: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
}

#[async_trait]
pub trait TotalsStore: Send + Sync {
    async fn global_daily_spend(&self) -> Result<Vec<DailySpend>, TotalsStoreError>;
    async fn key_daily_spend(&self, api_key: &str) -> Result<Vec<KeyDailySpend>, TotalsStoreError>;
    async fn user_daily_spend(
        &self,
        user_id: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<UserKeyDailySpend>, TotalsStoreError>;
    async fn global_total_spend(&self) -> Result<Vec<TotalSpendRow>, TotalsStoreError>;
    async fn keys_by_spend(&self, limit: Option<i64>) -> Result<Vec<KeySpend>, TotalsStoreError>;
    async fn user_keys_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<KeySpend>, TotalsStoreError>;
    async fn models_by_spend(&self, limit: i64) -> Result<Vec<ModelSpend>, TotalsStoreError>;
    async fn user_models_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<UserModelSpend>, TotalsStoreError>;
    async fn team_daily_spend(&self) -> Result<Vec<TeamSpend>, TotalsStoreError>;
}

pub struct PostgresTotalsStore {
    pub store: PostgresStore,
}

impl PostgresTotalsStore {
    async fn read<Spend>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<Spend>, TotalsStoreError>
    where
        Spend: for<'row> TryFrom<&'row Row, Error = TotalsStoreError>,
    {
        let client = self.store.session().await?;
        let rows = client
            .query(sql, params)
            .await
            .map_err(|source| TotalsStoreError::Database(backend(source)))?;
        rows.iter().map(Spend::try_from).collect()
    }
}

#[async_trait]
impl TotalsStore for PostgresTotalsStore {
    async fn global_daily_spend(&self) -> Result<Vec<DailySpend>, TotalsStoreError> {
        let sql = format!(
            "SELECT date, spend::text AS spend FROM \"{MONTHLY_GLOBAL_SPEND}\" ORDER BY date"
        );
        self.read(&sql, &[]).await
    }

    async fn key_daily_spend(&self, api_key: &str) -> Result<Vec<KeyDailySpend>, TotalsStoreError> {
        let sql = format!(
            "SELECT date, spend::text AS spend, api_key FROM \"{MONTHLY_GLOBAL_SPEND_PER_KEY}\" WHERE api_key = $1 ORDER BY date"
        );
        self.read(&sql, &[&api_key]).await
    }

    async fn user_daily_spend(
        &self,
        user_id: &str,
        api_key: Option<&str>,
    ) -> Result<Vec<UserKeyDailySpend>, TotalsStoreError> {
        let sql = format!(
            "SELECT date, spend::text AS spend, api_key, \"user\" FROM \"{MONTHLY_USER_KEY_SPEND}\" WHERE \"user\" = $1 AND ($2::text IS NULL OR api_key = $2) ORDER BY date"
        );
        self.read(&sql, &[&user_id, &api_key]).await
    }

    async fn global_total_spend(&self) -> Result<Vec<TotalSpendRow>, TotalsStoreError> {
        let sql = format!("SELECT SUM(spend)::text AS spend_text FROM \"{MONTHLY_GLOBAL_SPEND}\"");
        self.read(&sql, &[]).await
    }

    async fn keys_by_spend(&self, limit: Option<i64>) -> Result<Vec<KeySpend>, TotalsStoreError> {
        let sql = format!(
            "SELECT api_key, key_alias, key_name, total_spend::text AS spend_text FROM \"{LAST_30D_KEYS_BY_SPEND}\" ORDER BY total_spend DESC LIMIT $1"
        );
        self.read(&sql, &[&limit]).await
    }

    async fn user_keys_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<KeySpend>, TotalsStoreError> {
        self.read(USER_KEYS_SQL, &[&user_id, &limit]).await
    }

    async fn models_by_spend(&self, limit: i64) -> Result<Vec<ModelSpend>, TotalsStoreError> {
        let sql = format!(
            "SELECT model, total_spend::text AS spend_text FROM \"{LAST_30D_MODELS_BY_SPEND}\" ORDER BY total_spend DESC LIMIT $1"
        );
        self.read(&sql, &[&limit]).await
    }

    async fn user_models_by_spend(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<UserModelSpend>, TotalsStoreError> {
        self.read(USER_MODELS_SQL, &[&user_id, &limit]).await
    }

    async fn team_daily_spend(&self) -> Result<Vec<TeamSpend>, TotalsStoreError> {
        self.read(TEAM_DAILY_SQL, &[]).await
    }
}
