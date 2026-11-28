use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::{LiteLLMBudgetTableFull, Timestamp};
use betterlite_db::identity_rows::end_user::LiteLLMEndUserTable;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::UserRepository;
use betterlite_logging::prometheus::budget_gauges::inputs::{
    TeamBudgetGaugeInput, UserBudgetGaugeInput,
};
use betterlite_logging::prometheus::budget_gauges::lists::BudgetTableSnapshot;
use betterlite_logging::prometheus::budget_gauges::lists::customer_row::CustomerBudgetGaugeRow;
use betterlite_logging::prometheus::budget_gauges::lists::org_list::OrgBudgetGaugeRow;
use deadpool_postgres::Client;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use crate::key_query_plan::list::list_keys;
use crate::key_query_plan::response::KeyListEntry;
use crate::team_pagination::get_paginated_teams;

use super::source::{BudgetPageError, BudgetPageSource, Page};

use query::key_query;

mod query;
mod tests;

type PageResult<Row> = Result<Page<Row>, BudgetPageError>;

const USER_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_UserTable" t ORDER BY t."created_at" DESC OFFSET $1 LIMIT $2"#;
const USER_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_UserTable""#;
const TEAM_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_TeamTable""#;
const ORG_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_OrganizationTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" ORDER BY t."created_at" DESC OFFSET $1 LIMIT $2"#;
const ORG_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_OrganizationTable""#;
const CUSTOMER_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_EndUserTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" ORDER BY t."user_id" ASC OFFSET $1 LIMIT $2"#;
const CUSTOMER_PAGE_BUDGETED_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_EndUserTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" WHERE t."budget_id" IS NOT NULL ORDER BY t."user_id" ASC OFFSET $1 LIMIT $2"#;
const CUSTOMER_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_EndUserTable""#;
const CUSTOMER_COUNT_BUDGETED_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_EndUserTable" WHERE "budget_id" IS NOT NULL"#;

#[derive(Clone, Copy)]
struct TablePages {
    page: &'static str,
    count: Option<&'static str>,
}

const USER_PAGES: TablePages = TablePages {
    page: USER_PAGE_SQL,
    count: Some(USER_COUNT_SQL),
};
const ORG_PAGES: TablePages = TablePages {
    page: ORG_PAGE_SQL,
    count: Some(ORG_COUNT_SQL),
};

#[derive(Clone, Copy)]
pub struct PostgresBudgetPageSource<'db> {
    pub client: PrismaClient<'db>,
}

// B714 routes every Prisma read to the replica until its health flag says the replica is unavailable
fn read_store<'db>(client: PrismaClient<'db>) -> &'db PostgresStore {
    if client
        .availability
        .reader_unavailable
        .load(Ordering::SeqCst)
    {
        return client.writer;
    }
    client.reader
}

fn page_window(page_size: u64, page: u64) -> Result<(i64, i64), BudgetPageError> {
    let skipped = page
        .checked_sub(1)
        .and_then(|previous| previous.checked_mul(page_size))
        .ok_or(BudgetPageError::Window { page, page_size })?;
    let Ok(offset) = i64::try_from(skipped) else {
        return Err(BudgetPageError::Window { page, page_size });
    };
    let Ok(limit) = i64::try_from(page_size) else {
        return Err(BudgetPageError::Window { page, page_size });
    };
    Ok((offset, limit))
}

async fn count_rows(
    session: &Client,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<u64, BudgetPageError> {
    let row = session.query_one(sql, params).await.map_err(backend)?;
    let stored: i64 = row.try_get(0).map_err(backend)?;
    let count = u64::try_from(stored).map_err(|source| BudgetPageError::Count { source })?;
    Ok(count)
}

// LiteLLM's find_many/count pair: the page every time and, when asked, a fresh count of the same table
async fn page_rows(
    client: PrismaClient<'_>,
    page_size: u64,
    page: u64,
    pages: TablePages,
) -> PageResult<Row> {
    let (offset, limit) = page_window(page_size, page)?;
    let session = read_store(client).session().await?;
    let params: [&(dyn ToSql + Sync); 2] = [&offset, &limit];
    let rows = session.query(pages.page, &params).await.map_err(backend)?;
    let total_count = match pages.count {
        Some(sql) => Some(count_rows(&session, sql, &[]).await?),
        None => None,
    };
    Ok(Page { rows, total_count })
}

fn page_inputs<Row, Input>(
    page: Page<Row>,
    convert: fn(Row) -> Result<Input, BudgetPageError>,
) -> Result<Page<Input>, BudgetPageError> {
    let mut rows = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        rows.push(convert(row)?);
    }
    Ok(Page {
        rows,
        total_count: page.total_count,
    })
}

fn budgeted_inputs<Stored, Out>(
    page: Page<Row>,
    convert: fn(Stored, Option<BudgetTableSnapshot>) -> Out,
) -> Result<Page<Out>, BudgetPageError>
where
    Stored: serde::de::DeserializeOwned,
{
    let mut rows = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        let stored: Stored = stored_row(&row)?;
        rows.push(convert(stored, budget_of(&row)?));
    }
    Ok(Page {
        rows,
        total_count: page.total_count,
    })
}

fn stored_row<Stored: serde::de::DeserializeOwned>(row: &Row) -> Result<Stored, BudgetPageError> {
    serde_json::from_str(&row.try_get::<_, String>(0).map_err(backend)?)
        .map_err(BudgetPageError::Row)
}

fn team_input(team: LiteLLMTeamTable) -> TeamBudgetGaugeInput {
    TeamBudgetGaugeInput {
        team_id: team.fields.base.team_id,
        team_alias: team.fields.base.team_alias,
        spend: team.fields.spend,
        max_budget: team.fields.base.max_budget,
        budget_reset_at: team.fields.budget_reset_at.map(Timestamp::utc),
    }
}

fn user_row(row: Row) -> Result<UserBudgetGaugeInput, BudgetPageError> {
    let stored: LiteLLMUserTable = stored_row(&row)?;
    Ok(UserBudgetGaugeInput {
        user_id: stored.user_id,
        user_email: stored.user_email,
        user_alias: stored.user_alias,
        spend: stored.spend,
        max_budget: stored.max_budget,
        budget_reset_at: stored.budget_reset_at.map(Timestamp::utc),
    })
}

fn budget_of(row: &Row) -> Result<Option<BudgetTableSnapshot>, BudgetPageError> {
    let text: Option<String> = row.try_get(1).map_err(backend)?;
    let Some(text) = text else {
        return Ok(None);
    };
    let stored: LiteLLMBudgetTableFull =
        serde_json::from_str(&text).map_err(BudgetPageError::Row)?;
    Ok(Some(BudgetTableSnapshot {
        max_budget: stored.budget.max_budget,
        budget_reset_at: stored.budget_reset_at.map(Timestamp::utc),
    }))
}

fn org_row(stored: OrganizationTable, budget: Option<BudgetTableSnapshot>) -> OrgBudgetGaugeRow {
    OrgBudgetGaugeRow {
        organization_id: stored.organization_id,
        organization_alias: stored.organization_alias,
        spend: Some(stored.spend),
        budget,
    }
}

fn customer_row(
    stored: LiteLLMEndUserTable,
    budget: Option<BudgetTableSnapshot>,
) -> CustomerBudgetGaugeRow {
    CustomerBudgetGaugeRow {
        user_id: stored.user_id,
        spend: stored.spend,
        budget,
    }
}

fn customer_pages(budgeted_only: bool, page: u64) -> TablePages {
    let (page_sql, count_sql) = if budgeted_only {
        (CUSTOMER_PAGE_BUDGETED_SQL, CUSTOMER_COUNT_BUDGETED_SQL)
    } else {
        (CUSTOMER_PAGE_SQL, CUSTOMER_COUNT_SQL)
    };
    TablePages {
        page: page_sql,
        count: (page == 1).then_some(count_sql),
    }
}

#[async_trait]
impl BudgetPageSource for PostgresBudgetPageSource<'_> {
    async fn teams_page(&self, page_size: u64, page: u64) -> PageResult<TeamBudgetGaugeInput> {
        let (teams, total) = get_paginated_teams(self.client, page_size, page).await;
        Ok(Page {
            rows: teams.into_iter().map(team_input).collect(),
            total_count: Some(total),
        })
    }

    async fn keys_page(&self, page_size: u64, page: u64) -> PageResult<KeyListEntry> {
        let query = key_query(page_size, page)?;
        let response = list_keys(Some(self.client), &query).await?;
        let total_count = response
            .total_count
            .and_then(|count| u64::try_from(count).ok());
        Ok(Page {
            rows: response.keys,
            total_count,
        })
    }

    async fn users_page(&self, page_size: u64, page: u64) -> PageResult<UserBudgetGaugeInput> {
        let page = page_rows(self.client, page_size, page, USER_PAGES).await?;
        page_inputs(page, user_row)
    }

    async fn orgs_page(&self, page_size: u64, page: u64) -> PageResult<OrgBudgetGaugeRow> {
        let page = page_rows(self.client, page_size, page, ORG_PAGES).await?;
        budgeted_inputs(page, org_row)
    }

    async fn customers_page(
        &self,
        page_size: u64,
        page: u64,
        budgeted_only: bool,
    ) -> PageResult<CustomerBudgetGaugeRow> {
        let pages = customer_pages(budgeted_only, page);
        let page = page_rows(self.client, page_size, page, pages).await?;
        budgeted_inputs(page, customer_row)
    }

    async fn user_count(&self) -> Result<u64, BudgetPageError> {
        let session = read_store(self.client).session().await?;
        count_rows(&session, USER_COUNT_SQL, &[]).await
    }

    async fn billable_user_count(&self) -> Result<u64, BudgetPageError> {
        let session = read_store(self.client).session().await?;
        let users = UserRepository {
            db: Some(&session),
            writer_db: None,
            use_writer: false,
        };
        let stored = users.count_billable_users().await?;
        let count = u64::try_from(stored).map_err(|source| BudgetPageError::Count { source })?;
        Ok(count)
    }

    async fn team_count(&self) -> Result<u64, BudgetPageError> {
        let session = read_store(self.client).session().await?;
        count_rows(&session, TEAM_COUNT_SQL, &[]).await
    }
}
