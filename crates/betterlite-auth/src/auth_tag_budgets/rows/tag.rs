use std::convert::Infallible;

use betterlite_db::managed_resource_rows::tag::TagTable;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use tokio_postgres::Row;

use super::{BudgetInclude, NO_LIMIT, keyed_sql, listing, row_text};
use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_resource_grants::rows::RowError;

const TAG_LIST_BUDGET_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0'), 'litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_TagTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" LIMIT $1";

const TAG_LIST_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry LIMIT $1";

const TAG_IN_BUDGET_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0'), 'litellm_budget_table', CASE WHEN budget.\"budget_id\" IS NULL THEN NULL ELSE to_jsonb(budget) END))::text FROM \"LiteLLM_TagTable\" AS entry LEFT JOIN \"LiteLLM_BudgetTable\" AS budget ON budget.\"budget_id\" = entry.\"budget_id\" WHERE entry.\"tag_name\" = ANY($1::text[]) LIMIT $2";

const TAG_IN_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = ANY($1::text[]) LIMIT $2";

const TAG_ONE_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1";

const TAG_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = $1 RETURNING to_jsonb(entry)::text";

const TAG_UPDATE_SQL: &str = "UPDATE \"LiteLLM_TagTable\" AS entry SET \"tag_name\" = \"tag_name\" WHERE entry.\"tag_name\" = $1 RETURNING to_jsonb(entry)::text";

const TAG_INSERT_SQL: &str =
    "INSERT INTO \"LiteLLM_TagTable\" AS entry DEFAULT VALUES RETURNING to_jsonb(entry)::text";

const TAG_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_TagTable\"";

const TAG_MEMBERSHIP_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('models', COALESCE(to_jsonb(entry.\"models\"), '[]'::jsonb), 'spend', COALESCE(to_jsonb(entry.\"spend\"), '0')))::text FROM \"LiteLLM_TagTable\" AS entry WHERE entry.\"tag_name\" = ANY($1::text[])";

#[derive(Clone, Copy)]
pub struct TagBudgetTable;

impl RowCodec for TagBudgetTable {
    type Model = TagTable;
    type Error = RowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text = row_text(row)?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(RowError::Record)
    }
}

impl Table for TagBudgetTable {
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
            sql: keyed_sql(action, TAG_ONE_SQL, TAG_DELETE_SQL, TAG_UPDATE_SQL),
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
            sql: TAG_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&self, _filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        Query {
            sql: TAG_COUNT_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
        TAG_MEMBERSHIP_SQL
    }
}

impl Including for TagBudgetTable {
    type Include = BudgetInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let with_budget = include.is_some_and(|selected| selected.litellm_budget_table);
        match query {
            IncludingQuery::Keyed { action, key } => Query {
                sql: keyed_sql(action, TAG_ONE_SQL, TAG_DELETE_SQL, TAG_UPDATE_SQL),
                parameters: vec![key.value],
            },
            IncludingQuery::Listed { filter, limit } => listing(
                filter,
                limit,
                with_budget,
                [TAG_LIST_BUDGET_SQL, TAG_LIST_SQL],
                [TAG_IN_BUDGET_SQL, TAG_IN_SQL],
            ),
            IncludingQuery::Inserted { data } => Table::insert(self, data),
        }
    }
}
