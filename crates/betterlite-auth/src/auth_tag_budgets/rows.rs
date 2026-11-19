pub mod group;
pub mod tag;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::{KeyAction, Query};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

use crate::auth_read::table::AuthReadError;
use crate::auth_resource_grants::rows::RowError;

// One LiteLLM_ModelAccessGroupBudgetTable row with the budget relation Prisma includes beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAccessGroupBudgetRow {
    pub access_group_name: String,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub litellm_budget_table: Option<LiteLLMBudgetTable>,
}

// Prisma's include for both tables: only the budget relation is ever selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BudgetInclude {
    pub litellm_budget_table: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("the tag budget row lookup failed")]
pub struct TagBudgetRowsFailure {
    #[source]
    pub source: Box<AuthReadError<RowError>>,
}

#[async_trait]
pub trait TagRows: Send + Sync {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<TagTable>, TagBudgetRowsFailure>;
    async fn registry_rows(&self, take: i64) -> Result<Vec<TagTable>, TagBudgetRowsFailure>;
}

#[async_trait]
pub trait GroupRows: Send + Sync {
    async fn by_names(
        &self,
        names: Vec<String>,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure>;
    async fn registry_rows(
        &self,
        take: i64,
    ) -> Result<Vec<ModelAccessGroupBudgetRow>, TagBudgetRowsFailure>;
}

// Prisma's absent take means no limit, and Postgres returns the same rows for the largest row count.
const NO_TAKE: i64 = i64::MAX;
const NO_LIMIT: Option<i64> = None;

fn row_text(row: &Row) -> Result<String, RowError> {
    row.try_get(0)
        .map_err(|driver| RowError::Column(DriverError { driver }))
}

const fn keyed_sql(
    action: KeyAction<'_, ()>,
    one: &'static str,
    delete: &'static str,
    update: &'static str,
) -> &'static str {
    match action {
        KeyAction::Find => one,
        KeyAction::Delete => delete,
        KeyAction::Update(()) => update,
    }
}

fn listing<'query>(
    filter: Option<&'query Vec<String>>,
    limit: &'query Option<i64>,
    with_budget: bool,
    with_take: [&'static str; 2],
    in_list: [&'static str; 2],
) -> Query<'query> {
    let take = limit.unwrap_or(&NO_TAKE);
    let Some(names) = filter else {
        return Query {
            sql: if with_budget {
                with_take[0]
            } else {
                with_take[1]
            },
            parameters: vec![take],
        };
    };
    Query {
        sql: if with_budget { in_list[0] } else { in_list[1] },
        parameters: vec![names, take],
    }
}
