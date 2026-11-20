use betterlite_db::postgres_store::errors::backend;
use tokio_postgres::Row;

use super::error::AnalyticsError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveUsersReport {
    Daily,
    Weekly,
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
