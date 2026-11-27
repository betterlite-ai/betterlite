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

fn team_input(team: LiteLLMTeamTable) -> TeamBudgetGaugeInput {
    TeamBudgetGaugeInput {
        team_id: team.fields.base.team_id,
        team_alias: team.fields.base.team_alias,
        spend: team.fields.spend,
        max_budget: team.fields.base.max_budget,
        budget_reset_at: team.fields.budget_reset_at.map(Timestamp::utc),
    }
}

fn user_input(stored: LiteLLMUserTable) -> UserBudgetGaugeInput {
    UserBudgetGaugeInput {
        user_id: stored.user_id,
        user_email: stored.user_email,
        user_alias: stored.user_alias,
        spend: stored.spend,
        max_budget: stored.max_budget,
        budget_reset_at: stored.budget_reset_at.map(Timestamp::utc),
    }
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

fn org_row(row: &Row) -> Result<OrgBudgetGaugeRow, BudgetPageError> {
    let text: String = row.try_get(0).map_err(backend)?;
    let stored: OrganizationTable = serde_json::from_str(&text).map_err(BudgetPageError::Row)?;
    Ok(OrgBudgetGaugeRow {
        organization_id: stored.organization_id,
        organization_alias: stored.organization_alias,
        spend: Some(stored.spend),
        budget: budget_of(row)?,
    })
}

fn customer_row(row: &Row) -> Result<CustomerBudgetGaugeRow, BudgetPageError> {
    let text: String = row.try_get(0).map_err(backend)?;
    let stored: LiteLLMEndUserTable = serde_json::from_str(&text).map_err(BudgetPageError::Row)?;
    Ok(CustomerBudgetGaugeRow {
        user_id: stored.user_id,
        spend: stored.spend,
        budget: budget_of(row)?,
    })
}

fn customer_sql(budgeted_only: bool) -> (&'static str, &'static str) {
    if budgeted_only {
        (CUSTOMER_PAGE_BUDGETED_SQL, CUSTOMER_COUNT_BUDGETED_SQL)
    } else {
        (CUSTOMER_PAGE_SQL, CUSTOMER_COUNT_SQL)
    }
}

#[async_trait]
impl BudgetPageSource for PostgresBudgetPageSource<'_> {
    async fn teams_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<TeamBudgetGaugeInput>, BudgetPageError> {
        let (teams, total) = get_paginated_teams(self.client, page_size, page).await;
        let rows = teams.into_iter().map(team_input).collect();
        Ok(Page {
            rows,
            total_count: Some(total),
        })
    }

    async fn keys_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<KeyListEntry>, BudgetPageError> {
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

    async fn users_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<UserBudgetGaugeInput>, BudgetPageError> {
        let (offset, limit) = page_window(page_size, page)?;
        let session = read_store(self.client).session().await?;
        let params: [&(dyn ToSql + Sync); 2] = [&offset, &limit];
        let rows = session
            .query(USER_PAGE_SQL, &params)
            .await
            .map_err(backend)?;
        let total = count_rows(&session, USER_COUNT_SQL, &[]).await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in &rows {
            let text: String = row.try_get(0).map_err(backend)?;
            let stored: LiteLLMUserTable =
                serde_json::from_str(&text).map_err(BudgetPageError::Row)?;
            users.push(user_input(stored));
        }
        Ok(Page {
            rows: users,
            total_count: Some(total),
        })
    }

    async fn orgs_page(
        &self,
        page_size: u64,
        page: u64,
    ) -> Result<Page<OrgBudgetGaugeRow>, BudgetPageError> {
        let (offset, limit) = page_window(page_size, page)?;
        let session = read_store(self.client).session().await?;
        let params: [&(dyn ToSql + Sync); 2] = [&offset, &limit];
        let rows = session
            .query(ORG_PAGE_SQL, &params)
            .await
            .map_err(backend)?;
        let total = count_rows(&session, ORG_COUNT_SQL, &[]).await?;
        let mut orgs = Vec::with_capacity(rows.len());
        for row in &rows {
            orgs.push(org_row(row)?);
        }
        Ok(Page {
            rows: orgs,
            total_count: Some(total),
        })
    }

    async fn customers_page(
        &self,
        page_size: u64,
        page: u64,
        budgeted_only: bool,
    ) -> Result<Page<CustomerBudgetGaugeRow>, BudgetPageError> {
        let (offset, limit) = page_window(page_size, page)?;
        let (page_sql, count_sql) = customer_sql(budgeted_only);
        let session = read_store(self.client).session().await?;
        let params: [&(dyn ToSql + Sync); 2] = [&offset, &limit];
        let rows = session.query(page_sql, &params).await.map_err(backend)?;
        let mut customers = Vec::with_capacity(rows.len());
        for row in &rows {
            customers.push(customer_row(row)?);
        }
        let total_count = if page == 1 {
            Some(count_rows(&session, count_sql, &[]).await?)
        } else {
            None
        };
        Ok(Page {
            rows: customers,
            total_count,
        })
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
