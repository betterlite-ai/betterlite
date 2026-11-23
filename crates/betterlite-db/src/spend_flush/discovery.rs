use std::error::Error as StdError;
use std::future::Future;
use std::pin::Pin;

use crate::key_user_writes::retry::WriteClock;
use crate::postgres_store::errors::DbError;
use crate::postgres_store::session::PostgresStore;
use crate::tool_discovery::{ToolDiscoveryItem, ToolDiscoveryQueue};
use crate::tool_policy_store::deadline::StatementBounds;
use crate::tool_policy_store::discovery::batch_upsert_tools;
use crate::tool_policy_store::records::ToolDiscoveryItem as ToolRecord;
use crate::tool_policy_store::records::ToolIdGenerator;

mod tests;

pub type DiscoveryFuture<'run, Failure> =
    Pin<Box<dyn Future<Output = Result<(), Failure>> + Send + 'run>>;

// batch_upsert_tools over the tools one drain took: a failure is the drain's to log, and the drained tools never go back on the queue.
pub trait ToolDiscoveryWriter: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn upsert_tools<'run>(
        &'run self,
        items: &'run [ToolDiscoveryItem],
    ) -> DiscoveryFuture<'run, Self::Error>;
}

pub struct PostgresToolDiscovery<'run> {
    pub store: PostgresStore,
    pub ids: &'run dyn ToolIdGenerator,
    pub bounds: StatementBounds<'run>,
    pub clock: &'run dyn WriteClock,
}

impl ToolDiscoveryWriter for PostgresToolDiscovery<'_> {
    type Error = DbError;

    fn upsert_tools<'run>(
        &'run self,
        items: &'run [ToolDiscoveryItem],
    ) -> DiscoveryFuture<'run, DbError> {
        Box::pin(async move {
            let now = self.clock.now();
            let client = self.store.session().await?;
            let records = record_items(items);
            batch_upsert_tools(Some(client), self.ids, self.bounds, &records, now).await;
            Ok(())
        })
    }
}

// _flush_tool_discovery_queue: one drain, one batch upsert, and a failure is debug-logged without restoring a single tool.
pub async fn flush_tool_discovery_queue<Writer>(queue: &ToolDiscoveryQueue, writer: &Writer)
where
    Writer: ToolDiscoveryWriter,
{
    let items = queue.flush();
    if items.is_empty() {
        return;
    }
    if let Err(error) = writer.upsert_tools(&items).await {
        tracing::debug!("_flush_tool_discovery_queue error (non-blocking): {error}");
    }
}

fn record_items(items: &[ToolDiscoveryItem]) -> Vec<ToolRecord> {
    items
        .iter()
        .map(|item| ToolRecord {
            tool_name: item.tool_name.clone(),
            origin: item.origin.clone(),
            created_by: item.created_by.clone(),
            key_hash: item.key_hash.clone(),
            team_id: item.team_id.clone(),
            key_alias: item.key_alias.clone(),
            user_agent: item.user_agent.clone(),
        })
        .collect()
}
