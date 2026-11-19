use tokio_postgres::Row;

use crate::budget_cascade::enduser_invalidation::enduser_invalidation_match;
use crate::budget_cascade::linked_rows::{EndUserRow, LinkedKeys as _};
use crate::budget_reset_policy::cache_invalidation::{ResetCache, invalidate_caches};
use crate::budget_reset_policy::retry::RetryMode;
use crate::database_faults::DbError;
use crate::database_faults::reconnect::{DbReconnect, call_with_db_reconnect_retry};
use crate::identity_budget_reset::faults::read_fault;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::PostgresStore;

use super::enduser_page_truncation::EndUserWalk;

pub(crate) const ENDUSER_READ_REASON: &str = "reset_budget_read_endusers_failure";

// _enduser_invalidation_where without the spend filter: post-commit the zeroed rows match nobody, so the walk reads every customer the tier reached.
const LINKED_ENDUSER_PAGE_SQL: &str = "SELECT user_id, budget_id, spend::numeric AS spend FROM \"LiteLLM_EndUserTable\" WHERE budget_id = ANY($1::text[]) AND user_id > $2 ORDER BY user_id ASC LIMIT $3";
const DEFAULT_ENDUSER_PAGE_SQL: &str = "SELECT user_id, budget_id, spend::numeric AS spend FROM \"LiteLLM_EndUserTable\" WHERE (budget_id = ANY($1::text[]) OR budget_id IS NULL) AND user_id > $2 ORDER BY user_id ASC LIMIT $3";

#[derive(Debug, Clone, Copy)]
pub(crate) struct EndUserWalkRequest<'request> {
    pub(crate) budget_ids: &'request [String],
    pub(crate) default_enduser_budget_id: Option<&'request str>,
    pub(crate) batch_size: usize,
}

#[derive(Debug, Clone, Copy)]
struct EndUserPage<'request> {
    sql: &'static str,
    budget_ids: &'request [String],
    cursor: &'request str,
    limit: usize,
}

// _invalidate_enduser_caches: keyset-page every customer the committed reset touched, dropping each page's cached spend before reading the next.
pub(crate) async fn invalidate_enduser_caches<Cache, Reconnect>(
    store: &PostgresStore,
    cache: &Cache,
    request: EndUserWalkRequest<'_>,
    mut reconnect: Option<&mut Reconnect>,
) -> EndUserWalk
where
    Cache: ResetCache,
    Reconnect: DbReconnect + Send,
{
    if request.budget_ids.is_empty() {
        return EndUserWalk {
            cursor: None,
            invalidated: 0,
            truncated: false,
        };
    }
    let matched = enduser_invalidation_match(request.budget_ids, request.default_enduser_budget_id);
    let sql = if matched.include_unlinked {
        DEFAULT_ENDUSER_PAGE_SQL
    } else {
        LINKED_ENDUSER_PAGE_SQL
    };
    let mut walk = EndUserWalk {
        cursor: Some(String::new()),
        invalidated: 0,
        truncated: false,
    };
    while let Some(cursor) = walk.cursor.clone() {
        let page = EndUserPage {
            sql,
            budget_ids: request.budget_ids,
            cursor: &cursor,
            limit: request.batch_size,
        };
        let next =
            invalidate_enduser_page(store, cache, &page, &walk, reconnect.as_deref_mut()).await;
        walk = next;
    }
    walk
}

// _invalidate_enduser_page: one page's read and cache drops, or the failure branch that drops the cursor past the customers it reached.
async fn invalidate_enduser_page<Cache, Reconnect>(
    store: &PostgresStore,
    cache: &Cache,
    page: &EndUserPage<'_>,
    walk: &EndUserWalk,
    reconnect: Option<&mut Reconnect>,
) -> EndUserWalk
where
    Cache: ResetCache,
    Reconnect: DbReconnect + Send,
{
    match read_enduser_page(store, page, reconnect).await {
        Ok(rows) => {
            let last_user_id = rows.last().map(|row| row.user_id.as_str());
            let counter_keys: Vec<String> = rows.iter().map(EndUserRow::counter_key).collect();
            let cache_keys: Vec<String> = rows.iter().flat_map(EndUserRow::cache_keys).collect();
            invalidate_caches(cache, &counter_keys, &cache_keys).await;
            walk.after_page(last_user_id, rows.len(), page.limit)
        }
        Err(error) => {
            tracing::warn!(
                customers = walk.invalidated,
                cursor = %page.cursor,
                error = %error,
                "Failed to read a customer page for cache invalidation; the customers past it keep their cached spend until it expires"
            );
            walk.truncated_at()
        }
    }
}

async fn read_enduser_page<Reconnect>(
    store: &PostgresStore,
    page: &EndUserPage<'_>,
    reconnect: Option<&mut Reconnect>,
) -> Result<Vec<EndUserRow>, DbError>
where
    Reconnect: DbReconnect + Send,
{
    let policy = RetryMode::Read.policy(ENDUSER_READ_REASON);
    let attempt = move || fetch_enduser_page(store, page);
    call_with_db_reconnect_retry(reconnect, attempt, policy).await
}

async fn fetch_enduser_page(
    store: &PostgresStore,
    page: &EndUserPage<'_>,
) -> Result<Vec<EndUserRow>, DbError> {
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    let bound = i64::try_from(page.limit).unwrap_or(i64::MAX);
    let rows = client
        .query(page.sql, &[&page.budget_ids, &page.cursor, &bound])
        .await
        .map_err(|source| read_fault(&backend(source)))?;
    let mut customers = Vec::with_capacity(rows.len());
    for row in &rows {
        customers.push(decode_enduser(row)?);
    }
    Ok(customers)
}

fn decode_enduser(row: &Row) -> Result<EndUserRow, DbError> {
    Ok(EndUserRow {
        user_id: row
            .try_get("user_id")
            .map_err(|source| read_fault(&backend(source)))?,
        budget_id: row
            .try_get("budget_id")
            .map_err(|source| read_fault(&backend(source)))?,
        spend: row
            .try_get("spend")
            .map_err(|source| read_fault(&backend(source)))?,
    })
}

mod tests;
