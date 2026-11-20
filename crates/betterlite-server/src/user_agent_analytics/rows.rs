use betterlite_db::postgres_store::errors::backend;
use rust_decimal::Decimal;
use tokio_postgres::Row;
use tokio_postgres::types::FromSql;

use super::error::AnalyticsError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveUsersReport {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistinctTagRow {
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveUsersRow {
    pub tag: String,
    pub active_users: i64,
    pub date: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagSummaryRow {
    pub tag: String,
    pub unique_users: i64,
    pub total_requests: Option<i64>,
    pub successful_requests: Option<i64>,
    pub failed_requests: Option<i64>,
    pub total_tokens: Option<i64>,
    pub total_spend: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TagSpendRecord {
    pub tag: Option<String>,
    pub api_key: String,
    pub api_requests: Option<i64>,
    pub successful_requests: Option<i64>,
    pub failed_requests: Option<i64>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub spend: Option<Decimal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenOwnerRow {
    pub token: String,
    pub user_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserEmailRow {
    pub user_id: String,
    pub user_email: Option<String>,
}

fn column<T>(row: &Row, name: &'static str) -> Result<T, AnalyticsError>
where
    T: for<'row> FromSql<'row>,
{
    row.try_get(name)
        .map_err(|source| AnalyticsError::Database(backend(source)))
}

impl TryFrom<&Row> for TagSummaryRow {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            tag: column(row, "tag")?,
            unique_users: column(row, "unique_users")?,
            total_requests: column(row, "total_requests")?,
            successful_requests: column(row, "successful_requests")?,
            failed_requests: column(row, "failed_requests")?,
            total_tokens: column(row, "total_tokens")?,
            total_spend: column(row, "total_spend")?,
        })
    }
}

impl TryFrom<&Row> for TagSpendRecord {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            tag: column(row, "tag")?,
            api_key: column(row, "api_key")?,
            api_requests: column(row, "api_requests")?,
            successful_requests: column(row, "successful_requests")?,
            failed_requests: column(row, "failed_requests")?,
            prompt_tokens: column(row, "prompt_tokens")?,
            completion_tokens: column(row, "completion_tokens")?,
            spend: column(row, "spend")?,
        })
    }
}

impl TryFrom<&Row> for TokenOwnerRow {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            token: column(row, "token")?,
            user_id: column(row, "user_id")?,
        })
    }
}

impl TryFrom<&Row> for UserEmailRow {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        Ok(Self {
            user_id: column(row, "user_id")?,
            user_email: column(row, "user_email")?,
        })
    }
}

impl TryFrom<&Row> for DistinctTagRow {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        let tag = row
            .try_get("tag")
            .map_err(|source| AnalyticsError::Database(backend(source)))?;
        Ok(Self { tag })
    }
}

impl TryFrom<&Row> for ActiveUsersRow {
    type Error = AnalyticsError;

    fn try_from(row: &Row) -> Result<Self, Self::Error> {
        let tag = row
            .try_get("tag")
            .map_err(|source| AnalyticsError::Database(backend(source)))?;
        let active_users = row
            .try_get("active_users")
            .map_err(|source| AnalyticsError::Database(backend(source)))?;
        let date = row
            .try_get("date")
            .map_err(|source| AnalyticsError::Database(backend(source)))?;
        Ok(Self {
            tag,
            active_users,
            date,
        })
    }
}
