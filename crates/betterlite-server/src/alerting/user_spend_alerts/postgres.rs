use async_trait::async_trait;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{Datelike as _, Days, NaiveDate};
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, ToSql};

use super::{UserSpendRow, UserSpendSource};

// USER_SPEND_QUERY from user_spend_alerts.py: date is a text column, so all three parameters are text dates
const USER_SPEND_QUERY: &str = r#"
SELECT
    user_id,
    COALESCE(SUM(spend) FILTER (WHERE date = $1), 0)::float AS daily_spend,
    COALESCE(SUM(spend) FILTER (WHERE date >= $2), 0)::float AS monthly_spend,
    COALESCE(SUM(spend) FILTER (WHERE date >= $3 AND date < $1), 0)::float AS baseline_spend
FROM "LiteLLM_DailyUserSpend"
WHERE date >= LEAST($2, $3) AND user_id IS NOT NULL
GROUP BY user_id
HAVING COALESCE(SUM(spend) FILTER (WHERE date >= $2), 0) > 0
"#;

#[derive(Debug, thiserror::Error)]
pub enum UserSpendQueryError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error("the user spend window starts before the calendar supports")]
    Window,
    #[error("the {name} column holds a value betterlite cannot represent")]
    Unrepresentable { name: &'static str },
}

pub struct PgUserSpendSource {
    pub store: PostgresStore,
}

#[async_trait]
impl UserSpendSource for PgUserSpendSource {
    type Error = UserSpendQueryError;

    async fn rows(
        &self,
        today: NaiveDate,
        baseline_days: u32,
    ) -> Result<Vec<UserSpendRow>, Self::Error> {
        let today_text = today.format("%Y-%m-%d").to_string();
        let month_start = today
            .with_day(1)
            .ok_or(UserSpendQueryError::Window)?
            .format("%Y-%m-%d")
            .to_string();
        let baseline_start = today
            .checked_sub_days(Days::new(u64::from(baseline_days.max(1))))
            .ok_or(UserSpendQueryError::Window)?
            .format("%Y-%m-%d")
            .to_string();
        let client = self.store.session().await?;
        let params: [&(dyn ToSql + Sync); 3] = [&today_text, &month_start, &baseline_start];
        let rows = client
            .query(USER_SPEND_QUERY, &params)
            .await
            .map_err(|source| UserSpendQueryError::Database(backend(source)))?;
        rows.iter().map(decode).collect()
    }
}

// a Postgres ::float can be NaN or an infinity, and Decimal::from_f64_retain holds neither
fn decode(row: &Row) -> Result<UserSpendRow, UserSpendQueryError> {
    let daily: f64 = column(row, "daily_spend")?;
    let monthly: f64 = column(row, "monthly_spend")?;
    let baseline: f64 = column(row, "baseline_spend")?;
    Ok(UserSpendRow {
        user_id: column(row, "user_id")?,
        daily_spend: Decimal::from_f64_retain(daily).ok_or(
            UserSpendQueryError::Unrepresentable {
                name: "daily_spend",
            },
        )?,
        monthly_spend: Decimal::from_f64_retain(monthly).ok_or(
            UserSpendQueryError::Unrepresentable {
                name: "monthly_spend",
            },
        )?,
        baseline_spend: Decimal::from_f64_retain(baseline).ok_or(
            UserSpendQueryError::Unrepresentable {
                name: "baseline_spend",
            },
        )?,
    })
}

fn column<T>(row: &Row, name: &'static str) -> Result<T, UserSpendQueryError>
where
    T: for<'row> FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| UserSpendQueryError::Database(backend(source)))
}
