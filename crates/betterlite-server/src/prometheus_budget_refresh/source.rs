use std::error::Error as StdError;
use std::num::TryFromIntError;

use async_trait::async_trait;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::user_repository::RowDecodeError;
use betterlite_logging::prometheus::budget_gauges::inputs::{
    TeamBudgetGaugeInput, UserBudgetGaugeInput,
};
use betterlite_logging::prometheus::budget_gauges::lists::customer_row::CustomerBudgetGaugeRow;
use betterlite_logging::prometheus::budget_gauges::lists::org_list::OrgBudgetGaugeRow;

use crate::key_query_plan::KeyQueryPlanError;
use crate::key_query_plan::response::KeyListEntry;

// LiteLLM's _initialize_budget_metrics reads 50 rows per request
pub const PAGE_SIZE: u64 = 50;

#[derive(Debug)]
pub struct Page<Row> {
    pub rows: Vec<Row>,
    pub total_count: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetPageError {
    #[error("budget page {page} with page size {page_size} has no database offset")]
    Window { page: u64, page_size: u64 },
    #[error("the budget page count does not fit a u64")]
    Count {
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    BillableUsers(#[from] DbError<RowDecodeError>),
    #[error(transparent)]
    KeyQuery(#[from] KeyQueryPlanError),
    #[error("the stored budget row is not valid JSON")]
    Row(#[source] serde_json::Error),
    #[error("the budget gauge update failed: {source}")]
    Gauge {
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },
}

// One page of rows plus the count LiteLLM asks for with it; None means the caller counts the rows instead.
#[async_trait]
pub trait BudgetPageSource: Send + Sync {
    async fn teams_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<TeamBudgetGaugeInput>, BudgetPageError>;

    async fn keys_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<KeyListEntry>, BudgetPageError>;

    async fn users_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<UserBudgetGaugeInput>, BudgetPageError>;

    async fn orgs_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<OrgBudgetGaugeRow>, BudgetPageError>;

    async fn customers_page(
        &self,
        page_size: u64,
        page: u64,
        budgeted_only: bool,
    ) -> Result<Page<CustomerBudgetGaugeRow>, BudgetPageError>;

    async fn user_count(&self) -> Result<u64, BudgetPageError>;

    async fn billable_user_count(&self) -> Result<u64, BudgetPageError>;

    async fn team_count(&self) -> Result<u64, BudgetPageError>;
}
