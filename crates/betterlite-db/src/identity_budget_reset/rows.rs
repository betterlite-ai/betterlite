use std::convert::Infallible;

use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::GenericClient;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use tokio_postgres::Row;

use super::faults::{Fault, read_fault};
use super::key_reads;
use super::phase::IdentityPhase;
use super::user_reads;
use crate::postgres_store::errors::{DbError as NativeError, backend};
use crate::postgres_store::session::PostgresStore;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct DueIdentity {
    pub identifier: String,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub budget_duration: String,
    pub budget_reset_at: Option<DateTime<Utc>>,
}

pub(crate) async fn read_due(
    store: &PostgresStore,
    phase: IdentityPhase,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<DueIdentity>, Fault> {
    let (sql, id_column) = match phase {
        IdentityPhase::Keys => (key_reads::KEY_DUE_SQL, key_reads::KEY_ID_COLUMN),
        IdentityPhase::Users => (user_reads::USER_DUE_SQL, user_reads::USER_ID_COLUMN),
    };
    let client = store.session().await.map_err(|error| read_fault(&error))?;
    read_rows(&client, sql, id_column, now, limit)
        .await
        .map_err(|error| read_fault(&error))
}

pub(crate) async fn read_rows<C>(
    client: &C,
    sql: &str,
    id_column: &str,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<DueIdentity>, NativeError<Infallible>>
where
    C: GenericClient + Sync,
{
    let moment = now.naive_utc();
    let bound = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = client
        .query(sql, &[&moment, &bound])
        .await
        .map_err(backend)?;
    rows.iter()
        .map(|row| decode_due(row, id_column))
        .collect::<Result<Vec<DueIdentity>, NativeError<Infallible>>>()
}

// LiteLLM reads spend and max_budget as floats and floats the reset on them; a NULL or a value Decimal cannot hold reads as zero here.
fn decode_due(row: &Row, id_column: &str) -> Result<DueIdentity, NativeError<Infallible>> {
    let identifier: String = row.try_get(id_column).map_err(backend)?;
    let spend = row
        .try_get::<_, Option<f64>>("spend")
        .map_err(backend)?
        .and_then(Decimal::from_f64)
        .unwrap_or_default();
    let max_budget = row
        .try_get::<_, Option<f64>>("max_budget")
        .map_err(backend)?
        .and_then(Decimal::from_f64);
    let budget_duration: String = row.try_get("budget_duration").map_err(backend)?;
    let reset_at: Option<NaiveDateTime> = row.try_get("budget_reset_at").map_err(backend)?;
    Ok(DueIdentity {
        identifier,
        spend,
        max_budget,
        budget_duration,
        budget_reset_at: reset_at.map(NaiveDateTime::and_utc),
    })
}
