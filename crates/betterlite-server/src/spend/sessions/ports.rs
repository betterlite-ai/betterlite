use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::table_ops::rows::RowError;
use serde::Deserialize;

use crate::spend::log_results::projection::SpendLogListRow;

use super::params::SessionQuery;
use super::rows::SessionSpendRow;

// one session's keyset position: the page row the cursor is built from, and no Debug because api_key holds a key
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct SessionGroupRow {
    pub session_key: String,
    pub api_key: String,
    pub last_activity: String,
}

// _count_grouped_sessions and _count_spend_logs read one count row
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionGroupCount {
    pub total_count: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionsStoreError {
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    Row(#[from] RowError),
}

#[async_trait]
pub trait SessionsStore: Send + Sync {
    async fn session_groups(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionGroupRow>, SessionsStoreError>;
    async fn session_group_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError>;
    async fn session_representatives(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SpendLogListRow>, SessionsStoreError>;
    async fn session_rows(
        &self,
        query: &SessionQuery,
    ) -> Result<Vec<SessionSpendRow>, SessionsStoreError>;
    async fn session_row_count(&self, query: &SessionQuery) -> Result<i64, SessionsStoreError>;
}
