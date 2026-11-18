use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;

use super::reseed::SpendCounterReseed;
use super::rows::{SpendCounterRows, WindowEntity};
use super::values::{SpendValueError, aggregate_amount, stored_amount};

impl<Rows: SpendCounterRows> SpendCounterReseed<Rows> {
    pub async fn window_from_table(
        &self,
        entity_type: &CounterEntityType,
        entity_id: &str,
        window_duration: &str,
        expected_window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some(rows) = self.rows.as_ref() else {
            return Ok(None);
        };
        let Some(entity) = WindowEntity::from_counter_type(entity_type) else {
            return Ok(None);
        };
        match rows.window_row(entity, entity_id, window_duration).await {
            Ok(None) => Ok(None),
            Ok(Some(row)) => {
                if row.window_start.utc() < expected_window_start.utc() {
                    return Ok(None);
                }
                stored_amount(row.spend.as_ref()).map(Some)
            }
            Err(error) => {
                tracing::error!(%error, entity_id, "SpendCounterReseed.window_from_table: failed");
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
        if let Some(duration) = window_duration
            && let Some(from_table) = self
                .window_from_table(entity_type, entity_id, duration, window_start)
                .await?
        {
            return Ok(Some(from_table));
        }
        self.window_from_spend_logs(entity_type, entity_id, window_start)
            .await
    }

    pub async fn window_from_spend_logs(
        &self,
        entity_type: &CounterEntityType,
        entity_id: &str,
        window_start: Timestamp,
    ) -> Result<Option<Decimal>, SpendValueError> {
        let Some(rows) = self.rows.as_ref() else {
            return Ok(None);
        };
        let Some(entity) = WindowEntity::from_counter_type(entity_type) else {
            return Ok(None);
        };
        match rows
            .window_log_rows(entity, entity_id, window_start.utc())
            .await
        {
            Ok(found) => match found.first() {
                Some(row) => aggregate_amount(row).map(Some),
                None => Ok(Some(Decimal::ZERO)),
            },
            Err(error) => {
                tracing::error!(
                    %error,
                    entity_id,
                    "SpendCounterReseed.window_from_spend_logs: failed"
                );
                Ok(None)
            }
        }
    }
}
