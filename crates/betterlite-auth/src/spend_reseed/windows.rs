use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;

use super::reseed::SpendCounterReseed;
use super::rows::{SpendCounterRows, WindowEntity};
use super::values::{SpendValueError, aggregate_amount, stored_amount};

pub struct WindowQuery<'entity> {
    pub entity_type: &'entity CounterEntityType,
    pub entity_id: &'entity str,
    pub start: Timestamp,
}

impl<Rows: SpendCounterRows> SpendCounterReseed<Rows> {
    pub async fn window_from_table(
        &self,
        query: &WindowQuery<'_>,
        window_duration: &str,
        expected_window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some((rows, entity)) = self.window_source(query.entity_type) else {
            return Ok(None);
        };
        match rows
            .window_row(entity, query.entity_id, window_duration)
            .await
        {
            Ok(None) => Ok(None),
            Ok(Some(row)) => {
                if row.window_start.utc() < expected_window_start.utc() {
                    return Ok(None);
                }
                stored_amount(row.spend.as_ref()).map(Some)
            }
            Err(error) => {
                tracing::error!(%error, entity_id = query.entity_id, "SpendCounterReseed.window_from_table: failed");
                Ok(None)
            }
        }
    }

    pub async fn window_from_db(
        &self,
        entity_type: &CounterEntityType,
        entity_id: &str,
        window_duration: Option<&str>,
        window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let query = WindowQuery {
            entity_type,
            entity_id,
            start: window_start,
        };
        if let Some(duration) = window_duration {
            let from_table = self
                .window_from_table(&query, duration, window_start)
                .await?;
            if from_table.is_some() {
                return Ok(from_table);
            }
        }
        self.window_from_spend_logs(&query).await
    }

    pub async fn window_from_spend_logs(
        &self,
        query: &WindowQuery<'_>,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some((rows, entity)) = self.window_source(query.entity_type) else {
            return Ok(None);
        };
        let found = match rows
            .window_log_rows(entity, query.entity_id, query.start.utc())
            .await
        {
            Ok(found) => found,
            Err(error) => {
                tracing::error!(
                    %error,
                    entity_id = query.entity_id,
                    "SpendCounterReseed.window_from_spend_logs: failed"
                );
                return Ok(None);
            }
        };
        match found.first() {
            Some(row) => aggregate_amount(row).map(Some),
            None => Ok(Some(Decimal::ZERO)),
        }
    }

    fn window_source(&self, entity_type: &CounterEntityType) -> Option<(&Rows, WindowEntity)> {
        let rows = self.rows.as_deref()?;
        let entity = WindowEntity::from_counter_type(entity_type)?;
        Some((rows, entity))
    }
}
