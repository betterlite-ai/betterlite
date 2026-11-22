use crate::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use crate::spend_log_storage::{SpendLogWriteError as _, SpendLogsWriter};

mod tests;

// _create_spend_logs_with_poison_isolation's 256 attempts per group: how many failed inserts one 1000-row group may issue
pub const MAX_ISOLATION_FAILURES: u32 = 256;

// _create_spend_logs_with_poison_isolation: a data rejection bisects the statement so good rows still persist, and an outage never does
pub async fn create_logs_isolating_poison<Writer>(
    writer: &Writer,
    rows: &[SpendLogRow],
    failure_budget: u32,
) -> Result<u32, Writer::Error>
where
    Writer: SpendLogsWriter + ?Sized,
{
    let rejected = match writer.insert_rows(rows).await {
        Ok(_) => return Ok(failure_budget),
        Err(error) if !error.is_data_rejection() => return Err(error),
        Err(error) if error.is_service_unavailable() || error.is_deadlock() => return Err(error),
        Err(rejected) => rejected,
    };
    let budget_left = failure_budget.saturating_sub(1);
    if rows.len() == 1 {
        let poisoned = match rows.first().and_then(|row| row.get("request_id")) {
            Some(SpendLogValue::Text(id)) => id.as_str(),
            Some(_) | None => "None",
        };
        tracing::error!(
            "Spend tracking - dropping spend log row Postgres rejected. request_id={poisoned} error={rejected}"
        );
        return Ok(budget_left);
    }
    if budget_left == 0 {
        let abandoned = rows.len();
        tracing::error!(
            "Spend tracking - dropping {abandoned} spend log rows without per-row isolation; isolation failure budget exhausted for this flush"
        );
        return Ok(0);
    }
    let mid = rows.len().saturating_div(2);
    let (left, right) = rows.split_at(mid);
    let remaining = Box::pin(create_logs_isolating_poison(writer, left, budget_left)).await?;
    if remaining == 0 {
        let abandoned = right.len();
        tracing::error!(
            "Spend tracking - dropping {abandoned} spend log rows without per-row isolation; isolation failure budget exhausted for this flush"
        );
        return Ok(0);
    }
    Box::pin(create_logs_isolating_poison(writer, right, remaining)).await
}
