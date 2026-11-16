use std::error::Error;
use std::future::Future;

use betterlite_db::finops::focus::{FocusUsageError, get_usage_data};
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};

use crate::focus_records::transformer::UsageRow;

use super::rows::{self, UsageRowError};

pub trait UsageSource: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn usage_data(
        &self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<Vec<UsageRow>, Self::Error>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum UsageSourceError {
    #[error("the FOCUS usage data could not be queried")]
    Query(#[from] FocusUsageError),
    #[error("the FOCUS usage rows could not be decoded")]
    Rows(#[from] UsageRowError),
}

#[derive(Clone)]
pub struct PostgresUsageSource {
    pub store: PostgresStore,
}

impl UsageSource for PostgresUsageSource {
    type Error = UsageSourceError;

    async fn usage_data(
        &self,
        limit: Option<i64>,
        start_time_utc: Option<DateTime<Utc>>,
        end_time_utc: Option<DateTime<Utc>>,
    ) -> Result<Vec<UsageRow>, UsageSourceError> {
        let usage = get_usage_data(&self.store, limit, start_time_utc, end_time_utc).await?;
        Ok(rows::decode(&usage)?)
    }
}
