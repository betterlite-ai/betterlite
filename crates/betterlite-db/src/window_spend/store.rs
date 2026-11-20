use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use deadpool_postgres::GenericClient as _;
use rust_decimal::Decimal;
use tokio::time::timeout;
use tokio_postgres::types::ToSql;

use crate::postgres_store::session::PostgresStore;
use crate::window_queue::WindowSpendTransaction;

use super::error::{WindowSpendError, database};
use super::seed::{SeedEntity, SeedQuery, WindowSpendLogs, seed_base, window_instant};
use super::sql::{EXISTING_ROWS_SQL, ROLL_WINDOW_SPEND_SQL, UPSERT_WINDOW_SPEND_SQL};
use super::upsert::{PrimaryKey, WindowUpsertParams, primary_key, upsert_params};

pub const DEFAULT_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(60);

pub struct WindowRoll<'roll> {
    pub entity_type: &'roll str,
    pub entity_id: &'roll str,
    pub window_duration: &'roll str,
    pub new_window_start: NaiveDateTime,
    pub now: DateTime<Utc>,
}

pub struct WindowSpendStore {
    pub store: PostgresStore,
    pub transaction_timeout: Duration,
}

impl WindowSpendStore {
    #[must_use]
    pub const fn from_store(store: PostgresStore) -> Self {
        Self {
            store,
            transaction_timeout: DEFAULT_TRANSACTION_TIMEOUT,
        }
    }

    // commit_window_spend_updates: the existing-key read and every seed stay outside the transaction, which then applies all upserts under one injected now
    pub async fn commit_window_spend_updates<Logs: WindowSpendLogs>(
        &self,
        transactions: &[WindowSpendTransaction],
        logs: &Logs,
        now: DateTime<Utc>,
    ) -> Result<(), WindowSpendError> {
        if transactions.is_empty() {
            return Ok(());
        }
        let ordered = ordered_transactions(transactions);
        let existing = self.existing_primary_keys(&ordered).await?;
        let mut seeds = Vec::with_capacity(ordered.len());
        for transaction in &ordered {
            seeds.push(seed_for(transaction, &existing, logs).await?);
        }
        let instant = now.naive_utc();
        let params: Vec<WindowUpsertParams> = ordered
            .iter()
            .zip(&seeds)
            .map(|(transaction, base)| upsert_params(transaction, *base, instant))
            .collect::<Result<_, _>>()?;
        self.apply_upserts(&params).await
    }

    // roll_window_spend_row: conditional on the stored start still being behind the new one, so a row another pod already rolled keeps its spend
    pub async fn roll_window_spend_row(
        &self,
        roll: &WindowRoll<'_>,
    ) -> Result<(), WindowSpendError> {
        let now = roll.now.naive_utc();
        let values: [&(dyn ToSql + Sync); 5] = [
            &roll.entity_type,
            &roll.entity_id,
            &roll.window_duration,
            &roll.new_window_start,
            &now,
        ];
        let client = self.store.session().await?;
        client
            .execute(ROLL_WINDOW_SPEND_SQL, &values)
            .await
            .map_err(database)?;
        Ok(())
    }

    async fn existing_primary_keys(
        &self,
        transactions: &[&WindowSpendTransaction],
    ) -> Result<HashSet<PrimaryKey>, WindowSpendError> {
        let mut entity_types = Vec::with_capacity(transactions.len());
        let mut entity_ids = Vec::with_capacity(transactions.len());
        let mut window_durations = Vec::with_capacity(transactions.len());
        for transaction in transactions {
            entity_types.push(transaction.entity_type.clone());
            entity_ids.push(transaction.entity_id.clone());
            window_durations.push(transaction.window_duration.clone());
        }
        let values: [&(dyn ToSql + Sync); 3] = [&entity_types, &entity_ids, &window_durations];
        let client = self.store.session().await?;
        let rows = client
            .query(EXISTING_ROWS_SQL, &values)
            .await
            .map_err(database)?;
        let mut keys = HashSet::with_capacity(rows.len());
        for row in rows {
            let entity_type: String = row.try_get(0).map_err(database)?;
            let entity_id: String = row.try_get(1).map_err(database)?;
            let window_duration: String = row.try_get(2).map_err(database)?;
            keys.insert((entity_type, entity_id, window_duration));
        }
        Ok(keys)
    }

    async fn apply_upserts(&self, params: &[WindowUpsertParams]) -> Result<(), WindowSpendError> {
        let mut client = self.store.session().await?;
        let work = async {
            let transaction = client.transaction().await.map_err(database)?;
            for parameter in params {
                execute_upsert(&transaction, parameter).await?;
            }
            transaction.commit().await.map_err(database)
        };
        match timeout(self.transaction_timeout, work).await {
            Ok(result) => result,
            Err(_elapsed) => Err(WindowSpendError::TransactionTimeout {
                seconds: self.transaction_timeout.as_secs(),
            }),
        }
    }
}

// aggregate_window_spend's sort: the primary key first, then the window start, so an older window lands before the roll that supersedes it
pub(super) fn ordered_transactions(
    transactions: &[WindowSpendTransaction],
) -> Vec<&WindowSpendTransaction> {
    let mut ordered: Vec<&WindowSpendTransaction> = transactions.iter().collect();
    ordered.sort_by(|left, right| {
        (
            left.entity_type.as_str(),
            left.entity_id.as_str(),
            left.window_duration.as_str(),
            left.window_start.as_str(),
        )
            .cmp(&(
                right.entity_type.as_str(),
                right.entity_id.as_str(),
                right.window_duration.as_str(),
                right.window_start.as_str(),
            ))
    });
    ordered
}

// _seed_base_for_missing_row: an existing primary key gets no seed even when its stored window is older than the incoming one
async fn seed_for<Logs: WindowSpendLogs>(
    transaction: &WindowSpendTransaction,
    existing: &HashSet<PrimaryKey>,
    logs: &Logs,
) -> Result<Decimal, WindowSpendError> {
    if existing.contains(&primary_key(transaction)) {
        return Ok(Decimal::ZERO);
    }
    let Some(entity) = SeedEntity::from_litellm_name(&transaction.entity_type) else {
        return Ok(Decimal::ZERO);
    };
    let query = SeedQuery {
        entity,
        entity_id: &transaction.entity_id,
        window_start: window_instant(&transaction.window_start)?,
        batch_started_at: transaction
            .started_at
            .as_deref()
            .map(window_instant)
            .transpose()?,
    };
    let totals = logs.seed_totals(&query).await?;
    seed_base(&totals, transaction.spend.0)
}

async fn execute_upsert(
    transaction: &deadpool_postgres::Transaction<'_>,
    parameter: &WindowUpsertParams,
) -> Result<(), WindowSpendError> {
    let values: [&(dyn ToSql + Sync); 7] = [
        &parameter.entity_type,
        &parameter.entity_id,
        &parameter.window_duration,
        &parameter.window_start,
        &parameter.spend,
        &parameter.increment,
        &parameter.now,
    ];
    transaction
        .execute(UPSERT_WINDOW_SPEND_SQL, &values)
        .await
        .map_err(database)?;
    Ok(())
}
