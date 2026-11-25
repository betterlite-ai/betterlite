use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::rows::Query;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;

use super::rows::DailyTeamSpend;
use super::sql::SPEND_BY_TEAM_SQL;
use super::store::ScopedReportError;

// _query_raw_rows: Prisma's query_raw over B700's typed query, one JSON object text per row
pub async fn query_raw_rows(
    store: &PostgresStore,
    query: &Query<'_>,
) -> Result<Vec<String>, DbError> {
    let client = store.session().await?;
    let rows = client
        .query(query.sql, &query.parameters)
        .await
        .map_err(backend)?;
    let mut texts = Vec::with_capacity(rows.len());
    for row in &rows {
        texts.push(row.try_get(0).map_err(backend)?);
    }
    Ok(texts)
}

// the raw rows carry jsonb values, so the caller's typed model decodes each row's text
pub async fn decoded_rows<Row>(
    store: &PostgresStore,
    query: &Query<'_>,
) -> Result<Vec<Row>, ScopedReportError>
where
    Row: DeserializeOwned,
{
    let texts = query_raw_rows(store, query).await?;
    let mut rows = Vec::with_capacity(texts.len());
    for text in &texts {
        rows.push(serde_json::from_str(text)?);
    }
    Ok(rows)
}

// get_spend_by_team: the optional team filter is bound as NULL when the caller passes no team
pub async fn get_spend_by_team(
    store: &PostgresStore,
    start: &DateTime<Utc>,
    end: &DateTime<Utc>,
    team_id: Option<&str>,
) -> Result<Vec<DailyTeamSpend>, ScopedReportError> {
    let query = Query {
        sql: SPEND_BY_TEAM_SQL,
        parameters: vec![start, end, &team_id],
    };
    decoded_rows(store, &query).await
}
