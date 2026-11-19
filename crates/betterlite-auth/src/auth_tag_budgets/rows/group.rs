use std::convert::Infallible;

use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use tokio_postgres::Row;

use super::{BudgetInclude, ModelAccessGroupBudgetRow, NO_LIMIT, keyed_sql, listing, row_text};
use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_resource_grants::rows::RowError;

const GROUP_LIST_BUDGET_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" LIMIT $1";

const GROUP_LIST_SQL: &str =
    "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LIMIT $1";

const GROUP_IN_BUDGET_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" WHERE entry.\"access_group_name\" = ANY($1::text[]) LIMIT $2";

const GROUP_IN_SQL: &str = "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = ANY($1::text[]) LIMIT $2";

const GROUP_ONE_SQL: &str = "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = $1";

const GROUP_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = $1 RETURNING to_jsonb(entry)::text";

const GROUP_UPDATE_SQL: &str = "UPDATE \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry SET \"access_group_name\" = \"access_group_name\" WHERE entry.\"access_group_name\" = $1 RETURNING to_jsonb(entry)::text";

const GROUP_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry DEFAULT VALUES RETURNING to_jsonb(entry)::text";

const GROUP_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_ModelAccessGroupBudgetTable\"";

const GROUP_MEMBERSHIP_SQL: &str = "SELECT to_jsonb(entry)::text FROM \"LiteLLM_ModelAccessGroupBudgetTable\" AS entry WHERE entry.\"access_group_name\" = ANY($1::text[])";

#[derive(Clone, Copy)]
pub struct ModelAccessGroupBudgetTable;

impl RowCodec for ModelAccessGroupBudgetTable {
    type Model = ModelAccessGroupBudgetRow;
    type Error = RowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text = row_text(row)?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(RowError::Record)
    }
}

impl Table for ModelAccessGroupBudgetTable {
    type Column = Infallible;
    type Filter = Vec<String>;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        Query {
            sql: keyed_sql(action, GROUP_ONE_SQL, GROUP_DELETE_SQL, GROUP_UPDATE_SQL),
            parameters: vec![key.value],
        }
    }

    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        self.including(
            IncludingQuery::Listed {
                filter: options.filter.and_then(|filters| filters.first()),
                limit: &NO_LIMIT,
            },
            None,
        )
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: GROUP_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&self, _filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        Query {
            sql: GROUP_COUNT_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
        GROUP_MEMBERSHIP_SQL
    }
}

impl Including for ModelAccessGroupBudgetTable {
    type Include = BudgetInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let with_budget = include.is_some_and(|selected| selected.litellm_budget_table);
        match query {
            IncludingQuery::Keyed { action, key } => Query {
                sql: keyed_sql(action, GROUP_ONE_SQL, GROUP_DELETE_SQL, GROUP_UPDATE_SQL),
                parameters: vec![key.value],
            },
            IncludingQuery::Listed { filter, limit } => listing(
                filter,
                limit,
                with_budget,
                [GROUP_LIST_BUDGET_SQL, GROUP_LIST_SQL],
                [GROUP_IN_BUDGET_SQL, GROUP_IN_SQL],
            ),
            IncludingQuery::Inserted { data } => Table::insert(self, data),
        }
    }
}
