use chrono::{DateTime, Utc};
use tokio::time::timeout;
use tokio_postgres::Client;

use super::TRANSACTION_TIMEOUT;
use super::error::StoreError;
use super::sql::{DELETE_RETIRED, RETIRE_COMPARISONS};

// Old comparisons stop accepting estimates and lose their observations, while the savings they reported stay.
pub async fn retire_before(
    client: &mut Client,
    cutoff: DateTime<Utc>,
    batch_size: i64,
    timeout_ms: u64,
) -> Result<(), StoreError> {
    let limit = i32::try_from(batch_size)?;
    let bound = timeout_ms.max(1);
    let limits = format!("SET LOCAL statement_timeout = {bound}; SET LOCAL lock_timeout = {bound}");
    let retirement = async {
        let transaction = client.transaction().await?;
        transaction.batch_execute(&limits).await?;
        transaction
            .execute(RETIRE_COMPARISONS, &[&cutoff, &limit])
            .await?;
        transaction
            .execute(DELETE_RETIRED, &[&cutoff, &limit])
            .await?;
        transaction.commit().await
    };
    timeout(TRANSACTION_TIMEOUT, retirement).await??;
    Ok(())
}
