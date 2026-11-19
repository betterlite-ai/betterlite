use async_trait::async_trait;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio_postgres::Row;

use crate::auth_read::include::{Including, IncludingQuery};
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

// The two reads both budget tables need: the rows behind some names, and the bounded registry.
#[async_trait]
pub trait BudgetRows<Row>: Send + Sync {
    async fn by_names(&self, names: Vec<String>) -> Result<Vec<Row>, TagBudgetRowsFailure>;
    async fn registry_rows(&self, take: i64) -> Result<Vec<Row>, TagBudgetRowsFailure>;
}

// The name a registry load proves exists in the table it read.
pub trait RowName {
    fn row_name(&self) -> &str;
}

impl RowName for TagTable {
    fn row_name(&self) -> &str {
        &self.tag_name
    }
}

impl RowName for ModelAccessGroupBudgetRow {
    fn row_name(&self) -> &str {
        &self.access_group_name
    }
}

// Prisma's absent take means no limit, and Postgres returns the same rows for the largest row count.
const NO_TAKE: i64 = i64::MAX;

fn matched<'query>(
    filter: Option<&'query Vec<String>>,
    limit: &'query Option<i64>,
    with_budget: bool,
    with_take: [&'static str; 2],
    in_list: [&'static str; 2],
) -> Query<'query> {
    let take = limit.as_ref().unwrap_or(&NO_TAKE);
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

// the three impls both budget tables share: one plan name column and nine plans
macro_rules! budget_table {
    (
        $table:ident, $model:ty,
        one: $one:literal, delete: $delete:literal, update: $update:literal,
        insert: $insert:literal, count: $count:literal, membership: $membership:literal,
        list: [$list_budget:literal, $list:literal],
        matching: [$in_budget:literal, $in_rows:literal]
    ) => {
        #[derive(Clone, Copy)]
        pub struct $table;

        impl RowCodec for $table {
            type Model = $model;
            type Error = RowError;

            fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
                let text: String = row
                    .try_get(0)
                    .map_err(|driver| RowError::Column(DriverError { driver }))?;
                serde_json::from_str(&text)
                    .map(Some)
                    .map_err(RowError::Record)
            }
        }

        impl Table for $table {
            type Column = std::convert::Infallible;
            type Filter = Vec<String>;
            type Order = std::convert::Infallible;
            type Data = ();

            fn keyed<'query>(
                &'query self,
                action: KeyAction<'query, ()>,
                key: Key<'query, Self::Column>,
            ) -> Query<'query> {
                self.including(IncludingQuery::Keyed { action, key }, None)
            }

            fn list<'query>(
                &'query self,
                options: FindMany<'query, Self::Filter, Self::Order>,
            ) -> Query<'query> {
                self.including(
                    IncludingQuery::Listed {
                        filter: options.filter.and_then(|filters| filters.first()),
                        limit: &None,
                    },
                    None,
                )
            }

            fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
                Query {
                    sql: $insert,
                    parameters: Vec::new(),
                }
            }

            fn count<'query>(&self, _filter: Option<&'query [Self::Filter]>) -> Query<'query> {
                Query {
                    sql: $count,
                    parameters: Vec::new(),
                }
            }

            fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
                $membership
            }
        }

        impl Including for $table {
            type Include = super::BudgetInclude;

            fn including<'query>(
                &'query self,
                query: IncludingQuery<'query, Self>,
                include: Option<&'query Self::Include>,
            ) -> Query<'query> {
                let with_budget = include.is_some_and(|selected| selected.litellm_budget_table);
                match query {
                    IncludingQuery::Keyed { action, key } => Query {
                        sql: match action {
                            KeyAction::Find => $one,
                            KeyAction::Delete => $delete,
                            KeyAction::Update(()) => $update,
                        },
                        parameters: vec![key.value],
                    },
                    IncludingQuery::Listed { filter, limit } => super::matched(
                        filter,
                        limit,
                        with_budget,
                        [$list_budget, $list],
                        [$in_budget, $in_rows],
                    ),
                    IncludingQuery::Inserted { data } => Table::insert(self, data),
                }
            }
        }
    };
}

pub mod group;
pub mod tag;
