use std::collections::BTreeMap;

use betterlite_db::table_ops::rows::RowError;
use serde::Serialize;

use crate::spend::log_payload::metadata::hydrate_log_metadata;
use crate::spend::log_query::sql::SPEND_LOGS_COUNT_CAP;
use crate::spend::log_results::projection::{
    SpendLogListRow, SpendLogsPage, build_ui_spend_logs_response,
};
use crate::spend::log_results::session::{SessionAggregateError, SessionAggregateReader};

use super::cursor::{SessionCursor, decode_session_cursor};
use super::params::SessionParam;
use super::ports::{SessionGroupRow, SessionsStore, SessionsStoreError};

pub mod sql;

use self::sql::{GroupedPage, count_query, page_query, representative_query};

// the call B1732 makes once its filters are authorized: conditions and their positional values, the requested page, the direction, and the previous page's keyset position
pub struct GroupedSessionRequest<'query> {
    pub conditions: &'query [String],
    pub params: Vec<SessionParam>,
    pub page: i64,
    pub page_size: i64,
    pub descending: bool,
    pub session_cursor: Option<&'query str>,
}

// _ui_session_grouped_spend_logs' response: the flat page fields plus the keyset continuation
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SessionLogsPage {
    #[serde(flatten)]
    pub page: SpendLogsPage,
    pub next_session_cursor: Option<String>,
    pub has_more: bool,
}

// the window one grouped page covers and how many groups the query answered beyond it
struct PageWindow {
    offset: i64,
    limit: i64,
    fetched: i64,
    has_more: bool,
}

// _ui_session_grouped_spend_logs: one row per session, keyset-paginated by last activity, with each page's representatives fetched in that page's order and enriched like the flat listing
pub async fn ui_session_grouped_spend_logs(
    store: &dyn SessionsStore,
    enricher: Option<&dyn SessionAggregateReader>,
    request: &GroupedSessionRequest<'_>,
) -> Result<SessionLogsPage, SessionsStoreError> {
    let cursor = decode_session_cursor(request.session_cursor);
    let (group_rows, window) = fetch_window(store, request, cursor.as_ref()).await?;
    let visible: Vec<SessionGroupRow> = group_rows
        .into_iter()
        .take(usize::try_from(window.limit).unwrap_or(0))
        .collect();
    let next_session_cursor = next_cursor(&visible, window.has_more);
    let (total, total_is_capped) =
        total_records(store, request, cursor.as_ref(), &window, &visible).await?;
    let data = representatives(store, request, &visible).await?;
    let page = build_page(enricher, request, data, total, total_is_capped).await?;
    Ok(SessionLogsPage {
        page,
        next_session_cursor,
        has_more: window.has_more,
    })
}

// the offset page's window trimmed at the count cap, or the cursor page's keyset window, and the rows it answered
async fn fetch_window(
    store: &dyn SessionsStore,
    request: &GroupedSessionRequest<'_>,
    cursor: Option<&SessionCursor>,
) -> Result<(Vec<SessionGroupRow>, PageWindow), SessionsStoreError> {
    let offset = if cursor.is_some() {
        0
    } else {
        request
            .page
            .saturating_sub(1)
            .saturating_mul(request.page_size)
    };
    let remaining = SPEND_LOGS_COUNT_CAP.checked_sub(offset).unwrap_or(i64::MIN);
    let limit = request.page_size.min(remaining);
    let rows = if limit <= 0 {
        Vec::new()
    } else {
        store
            .session_groups(&page_query(&page_plan(
                request,
                cursor.cloned(),
                limit,
                offset,
            )))
            .await?
    };
    let fetched = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    let window = PageWindow {
        offset,
        limit,
        fetched,
        has_more: fetched > limit,
    };
    Ok((rows, window))
}

fn page_plan<'request>(
    request: &'request GroupedSessionRequest<'_>,
    cursor: Option<SessionCursor>,
    limit: i64,
    offset: i64,
) -> GroupedPage<'request> {
    GroupedPage {
        conditions: request.conditions,
        params: request.params.clone(),
        cursor,
        descending: request.descending,
        page_limit: limit,
        offset,
    }
}

// the last visible group as the next page's position, only when an extra group proved the list continues
fn next_cursor(visible: &[SessionGroupRow], has_more: bool) -> Option<String> {
    if !has_more {
        return None;
    }
    visible
        .last()
        .map(|row| format!("{}|{}|{}", row.last_activity, row.api_key, row.session_key))
}

// a noncursor page that runs out while holding groups ends the list and names the total itself, while a page past the end or a cursor page says nothing
async fn total_records(
    store: &dyn SessionsStore,
    request: &GroupedSessionRequest<'_>,
    cursor: Option<&SessionCursor>,
    window: &PageWindow,
    visible: &[SessionGroupRow],
) -> Result<(i64, bool), SessionsStoreError> {
    let starts_inside = window.offset == 0 || window.fetched > 0;
    let ends_the_list =
        cursor.is_none() && window.limit.is_positive() && !window.has_more && starts_inside;
    if ends_the_list {
        return Ok((window.offset.saturating_add(window.fetched), false));
    }
    let counted = store
        .session_group_count(&count_query(request.conditions, &request.params))
        .await?;
    if counted > SPEND_LOGS_COUNT_CAP {
        return Ok((SPEND_LOGS_COUNT_CAP, true));
    }
    Ok((counted, false))
}

async fn build_page(
    enricher: Option<&dyn SessionAggregateReader>,
    request: &GroupedSessionRequest<'_>,
    data: Vec<SpendLogListRow>,
    total: i64,
    total_is_capped: bool,
) -> Result<SpendLogsPage, SessionsStoreError> {
    let padded = total.checked_add(request.page_size.saturating_sub(1));
    let divided = padded.and_then(|sum| sum.checked_div(request.page_size));
    let page = SpendLogsPage {
        data,
        total,
        page: request.page,
        page_size: request.page_size,
        total_pages: divided.unwrap_or(i64::MAX),
        total_is_capped,
    };
    build_ui_spend_logs_response(enricher, page, true)
        .await
        .map_err(aggregate_failure)
}

// _fetch_session_representatives, returned in the page's group order, then hydrated
async fn representatives(
    store: &dyn SessionsStore,
    request: &GroupedSessionRequest<'_>,
    visible: &[SessionGroupRow],
) -> Result<Vec<SpendLogListRow>, SessionsStoreError> {
    let session_keys: Vec<(String, String)> = visible
        .iter()
        .map(|row| (row.session_key.clone(), row.api_key.clone()))
        .collect();
    if session_keys.is_empty() {
        return Ok(Vec::new());
    }
    let found = store
        .session_representatives(&representative_query(
            request.conditions,
            &request.params,
            &session_keys,
        ))
        .await?;
    let mut by_key: BTreeMap<(String, String), SpendLogListRow> = found
        .into_iter()
        .map(|row| (representative_key(&row), row))
        .collect();
    let mut data = Vec::with_capacity(session_keys.len());
    for key in session_keys {
        if let Some(row) = by_key.remove(&key) {
            data.push(row);
        }
    }
    for row in &mut data {
        if let Some(metadata) = row.metadata.as_mut() {
            hydrate_log_metadata(metadata);
        }
    }
    Ok(data)
}

// the pair the representative query selected, with the session key the row resolves to
fn representative_key(row: &SpendLogListRow) -> (String, String) {
    let session_key = row
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())
        .unwrap_or(&row.request_id);
    (
        session_key.to_owned(),
        row.api_key.clone().unwrap_or_default(),
    )
}

fn aggregate_failure(error: SessionAggregateError) -> SessionsStoreError {
    match error {
        SessionAggregateError::Database(source) => SessionsStoreError::Database(source),
        SessionAggregateError::Row(source) => SessionsStoreError::Row(RowError::Model(source)),
    }
}
