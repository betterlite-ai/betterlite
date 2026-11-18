use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use tokio_postgres::types::ToSql;

pub struct CapturedSpendRequest<'days> {
    pub providers: &'days [&'days str],
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

#[derive(Debug, thiserror::Error)]
pub enum CapturedSpendError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the captured spend for {date} is not a decimal")]
    UnreadableAmount {
        date: String,
        #[source]
        source: rust_decimal::Error,
    },
}

#[async_trait]
pub trait CapturedSpendSource: Send + Sync {
    async fn captured_by_day(
        &self,
        request: &CapturedSpendRequest<'_>,
    ) -> Result<BTreeMap<String, Decimal>, CapturedSpendError>;
}

pub struct PostgresCapturedSpend {
    pub store: PostgresStore,
}

// The source sums DOUBLE PRECISION; casting the sum to numeric keeps every cent the database holds.
const CAPTURED_SPEND_SQL: &str = "SELECT date, COALESCE(SUM(spend), 0)::numeric::text AS spend FROM \"LiteLLM_DailyUserSpend\" WHERE date >= $1 AND date <= $2 AND custom_llm_provider = ANY($3::text[]) GROUP BY date";

#[async_trait]
impl CapturedSpendSource for PostgresCapturedSpend {
    async fn captured_by_day(
        &self,
        request: &CapturedSpendRequest<'_>,
    ) -> Result<BTreeMap<String, Decimal>, CapturedSpendError> {
        let client = self.store.session().await?;
        let start = request.start_date.format("%Y-%m-%d").to_string();
        let end = request.end_date.format("%Y-%m-%d").to_string();
        let providers: Vec<String> = request
            .providers
            .iter()
            .map(|provider| (*provider).to_owned())
            .collect();
        let params: [&(dyn ToSql + Sync); 3] = [&start, &end, &providers];
        let rows = client
            .query(CAPTURED_SPEND_SQL, &params)
            .await
            .map_err(|source| CapturedSpendError::Database(backend(source)))?;
        let mut days = BTreeMap::new();
        for row in &rows {
            let date: String = row
                .try_get("date")
                .map_err(|source| CapturedSpendError::Database(backend(source)))?;
            let text: String = row
                .try_get("spend")
                .map_err(|source| CapturedSpendError::Database(backend(source)))?;
            let spend =
                text.parse::<Decimal>()
                    .map_err(|source| CapturedSpendError::UnreadableAmount {
                        date: date.clone(),
                        source,
                    })?;
            days.insert(date, spend);
        }
        Ok(days)
    }
}
