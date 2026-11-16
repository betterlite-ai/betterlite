use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::GenericClient;
use tokio_postgres::types::{ToSql, Type};
use uuid::Uuid;

use super::StoreError;
use super::sleeper::Sleeper;
use crate::daily_activity_sql::PTU_SENTINEL_API_KEY;
use crate::postgres_store::errors::backend;
use crate::ptu::charges::pricing::PtuCharge;

pub const UPSERT_ATTEMPTS: u32 = 3;
pub const RETRY_BACKOFF: Duration = Duration::from_millis(500);

const UPSERT_TYPES: [Type; 8] = [
    Type::TEXT,
    Type::TEXT,
    Type::TEXT,
    Type::TEXT,
    Type::TEXT,
    Type::TEXT,
    Type::NUMERIC,
    Type::TIMESTAMP,
];

const UPSERT_SQL: &str = "INSERT INTO \"LiteLLM_DailyTeamSpend\" \
    (\"id\", \"team_id\", \"date\", \"api_key\", \"model\", \"model_group\", \
     \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\", \
     \"ptu_flat_cost\", \"updated_at\") \
    VALUES ($1, $2, $3, $4, $5, $6, '', '', '', $7, $8) \
    ON CONFLICT (\"team_id\", \"date\", \"api_key\", \"model\", \
     \"custom_llm_provider\", \"mcp_namespaced_tool_name\", \"endpoint\") \
    DO UPDATE SET \"model_group\" = EXCLUDED.\"model_group\", \
     \"ptu_flat_cost\" = EXCLUDED.\"ptu_flat_cost\", \
     \"updated_at\" = EXCLUDED.\"updated_at\"";

const LOOKUP_SQL: &str = "SELECT \"team_id\", \"model\", \"date\" \
    FROM \"LiteLLM_DailyTeamSpend\" \
    WHERE \"api_key\" = $1 AND \"date\" >= $2 AND \"date\" <= $3";

pub struct ChargeWrite<'charge> {
    pub charge: &'charge PtuCharge,
    pub date: NaiveDate,
    pub id: Uuid,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SentinelKey {
    pub team_id: String,
    pub model: String,
    pub date: String,
}

#[must_use]
pub async fn write_charge_with_retry<C, S>(
    client: &C,
    sleeper: &mut S,
    write: &ChargeWrite<'_>,
) -> bool
where
    C: GenericClient + Sync,
    S: Sleeper + ?Sized,
{
    for attempt in 1..=UPSERT_ATTEMPTS {
        match upsert(client, write).await {
            Ok(()) => return true,
            Err(error) if attempt == UPSERT_ATTEMPTS => {
                tracing::error!(
                    "PTU rollup: upsert failed after {} attempts for team={} model={} day={} \
                     (rerun the rollup for that date to recover): {error}",
                    UPSERT_ATTEMPTS,
                    write.charge.team_id,
                    write.charge.model_name,
                    write.date,
                );
            }
            Err(error) => {
                tracing::warn!(
                    "PTU rollup: upsert attempt {}/{} failed for team={} model={} day={}: {error}",
                    attempt,
                    UPSERT_ATTEMPTS,
                    write.charge.team_id,
                    write.charge.model_name,
                    write.date,
                );
                sleeper.sleep(RETRY_BACKOFF.saturating_mul(attempt)).await;
            }
        }
    }
    false
}

pub async fn existing_sentinel_keys<C>(
    client: &C,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<BTreeSet<SentinelKey>, StoreError>
where
    C: GenericClient + Sync,
{
    let start = start.to_string();
    let end = end.to_string();
    let rows = client
        .query(LOOKUP_SQL, &[&PTU_SENTINEL_API_KEY, &start, &end])
        .await
        .map_err(|error| StoreError::Lookup(backend(error)))?;
    rows.iter()
        .map(sentinel_key)
        .collect::<Result<BTreeSet<SentinelKey>, StoreError>>()
}

fn sentinel_key(row: &tokio_postgres::Row) -> Result<SentinelKey, StoreError> {
    let team_id: Option<String> = row
        .try_get("team_id")
        .map_err(|error| StoreError::Lookup(backend(error)))?;
    let model: Option<String> = row
        .try_get("model")
        .map_err(|error| StoreError::Lookup(backend(error)))?;
    let date: Option<String> = row
        .try_get("date")
        .map_err(|error| StoreError::Lookup(backend(error)))?;
    Ok(SentinelKey {
        team_id: team_id.unwrap_or_default(),
        model: model.unwrap_or_default(),
        date: date.unwrap_or_default(),
    })
}

async fn upsert<C>(client: &C, write: &ChargeWrite<'_>) -> Result<(), StoreError>
where
    C: GenericClient + Sync,
{
    let id = write.id.to_string();
    let date = write.date.to_string();
    let flat_cost = write.charge.flat_cost;
    let updated_at = write.updated_at.naive_utc();
    let params: [&(dyn ToSql + Sync); 8] = [
        &id,
        &write.charge.team_id,
        &date,
        &PTU_SENTINEL_API_KEY,
        &write.charge.model_id,
        &write.charge.model_name,
        &flat_cost,
        &updated_at,
    ];
    let statement = client
        .prepare_typed(UPSERT_SQL, &UPSERT_TYPES)
        .await
        .map_err(|error| StoreError::Write(backend(error)))?;
    client
        .execute(&statement, &params)
        .await
        .map_err(|error| StoreError::Write(backend(error)))?;
    Ok(())
}
