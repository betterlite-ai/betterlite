use std::sync::Arc;

use betterlite_redis::list_operations::error::RedisListOperationsError;
use futures_util::future::BoxFuture;

use crate::gateway_requests::GatewayRequestKey;
use crate::gateway_requests::GatewayRequestSnapshot;
use crate::gateway_requests::gateway_request_counts::GatewayRequestCounts;

use super::commit_gateway_requests::GatewayRequestsDatabase;
use super::commit_gateway_requests::commit_gateway_requests_to_db;
use super::fold_gateway_counts::fold_counts;

pub mod adapters;
mod tests;

pub const GATEWAY_REQUESTS_JOB_NAME: &str = "update_gateway_requests_job";
pub const GATEWAY_REQUESTS_BUFFER_KEY: &str = "litellm_gateway_requests_buffer";
pub const MAX_REDIS_BUFFER_DEQUEUE_COUNT: usize = 100;

type BufferedRow = (String, String, String, i64, i64);

#[derive(Debug, thiserror::Error)]
pub enum GatewayRequestRedisError {
    #[error("the redis buffer {operation} failed")]
    List {
        operation: &'static str,
        #[source]
        source: RedisListOperationsError,
    },
    #[error("the buffered snapshot could not be written as JSON")]
    Encode(#[source] serde_json::Error),
}

pub type ListFuture<'call, Value> = BoxFuture<'call, Result<Value, GatewayRequestRedisError>>;

pub trait GatewayRequestsList: Send + Sync {
    fn append<'call>(&'call self, entry: &'call str) -> ListFuture<'call, ()>;

    fn take(&self, count: usize) -> ListFuture<'_, Vec<String>>;
}

pub type LeaseFuture<'call> = BoxFuture<'call, Option<bool>>;

pub trait GatewayRequestsLease: Send + Sync {
    fn acquire<'call>(&'call self, job: &'call str) -> LeaseFuture<'call>;
}

pub struct GatewayRequestRedisBuffer {
    pub list: Arc<dyn GatewayRequestsList>,
    pub lease: Arc<dyn GatewayRequestsLease>,
    pub dequeue_count: usize,
}

impl GatewayRequestRedisBuffer {
    pub async fn push(
        &self,
        snapshot: &GatewayRequestSnapshot,
    ) -> Result<(), GatewayRequestRedisError> {
        if snapshot.is_empty() {
            return Ok(());
        }
        let rows: Vec<BufferedRow> = snapshot
            .iter()
            .map(|(key, counts)| {
                (
                    key.date.clone(),
                    key.category.clone(),
                    key.route.clone(),
                    counts.successful_requests,
                    counts.failed_requests,
                )
            })
            .collect();
        let payload = serde_json::to_string(&rows).map_err(GatewayRequestRedisError::Encode)?;
        self.list.append(&payload).await
    }

    pub async fn pop(&self) -> Result<GatewayRequestSnapshot, GatewayRequestRedisError> {
        let entries = self.drain().await?;
        let mut rows: Vec<BufferedRow> = Vec::new();
        for entry in &entries {
            rows.extend(decode_row_list(entry));
        }
        Ok(fold_counts(rows.into_iter().map(|row| {
            let (date, category, route, successful_requests, failed_requests) = row;
            (
                GatewayRequestKey {
                    date,
                    category,
                    route,
                },
                GatewayRequestCounts {
                    successful_requests,
                    failed_requests,
                },
            )
        })))
    }

    async fn drain(&self) -> Result<Vec<String>, GatewayRequestRedisError> {
        let count = self.dequeue_count.max(1);
        let mut entries: Vec<String> = Vec::new();
        loop {
            let batch = match self.list.take(count).await {
                Ok(batch) => batch,
                Err(error) if entries.is_empty() => return Err(error),
                Err(error) => {
                    tracing::warn!(
                        "Gateway request tracking - Redis drain stopped after {} entries, the rest stay in Redis for the next flush: {error}",
                        entries.len()
                    );
                    break;
                }
            };
            let short = batch.len() < count;
            entries.extend(batch);
            if short {
                break;
            }
        }
        Ok(entries)
    }

    pub async fn commit_if_leader<Database>(
        &self,
        database: &Database,
    ) -> Result<GatewayRequestSnapshot, GatewayRequestRedisError>
    where
        Database: GatewayRequestsDatabase,
    {
        if self.lease.acquire(GATEWAY_REQUESTS_JOB_NAME).await != Some(true) {
            return Ok(GatewayRequestSnapshot::new());
        }
        let buffered = self.pop().await?;
        match commit_gateway_requests_to_db(database, &buffered).await {
            Ok(_committed) => Ok(GatewayRequestSnapshot::new()),
            Err(error) => {
                tracing::warn!(
                    "Gateway request tracking - failed to commit {} buffered rows, re-queuing to Redis for the next flush: {error}",
                    buffered.len()
                );
                Ok(self.requeue(&buffered).await)
            }
        }
    }

    async fn requeue(&self, snapshot: &GatewayRequestSnapshot) -> GatewayRequestSnapshot {
        if let Err(error) = self.push(snapshot).await {
            tracing::warn!(
                "Gateway request tracking - Redis re-queue failed, keeping {} rows in memory for the next flush: {error}",
                snapshot.len()
            );
            return snapshot.clone();
        }
        GatewayRequestSnapshot::new()
    }
}

fn decode_row_list(entry: &str) -> Vec<BufferedRow> {
    match serde_json::from_str(entry) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                "Gateway request tracking - dropped a buffered entry that is not a row list: {error}"
            );
            Vec::new()
        }
    }
}
