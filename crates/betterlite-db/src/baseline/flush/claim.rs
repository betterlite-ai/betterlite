use tokio::time::timeout;

use crate::baseline::store::error::StoreError;
use crate::baseline::store::sql::CLAIM_DIRTY;
use crate::baseline::store::{TRANSACTION_TIMEOUT, begin};
use crate::database_replica::pinned::{WriterPinnedClient, writer_wrapper};
use crate::postgres_store::session::PostgresStore;

// _CLAIM_DIRTY's LIMIT: a claim this full means more dirty scopes may be waiting.
pub const CLAIM_BATCH: usize = 32;
pub const CLAIM_LIMITS: &str = "SET LOCAL statement_timeout = 1000";

// The claim commits its attempted_at stamps before any projection locks the same rows on another connection.
pub async fn claim_dirty(
    database: WriterPinnedClient<'_, PostgresStore>,
) -> Result<Vec<String>, StoreError> {
    let claim = async {
        let mut session = writer_wrapper(database).session().await?;
        let transaction = begin(&mut session, CLAIM_LIMITS).await?;
        let rows = transaction.query(CLAIM_DIRTY, &[]).await?;
        let scopes = rows
            .iter()
            .map(|row| row.try_get("scope"))
            .collect::<Result<Vec<String>, _>>()?;
        transaction.commit().await?;
        Ok::<_, StoreError>(scopes)
    };
    timeout(TRANSACTION_TIMEOUT, claim).await?
}
