use std::sync::atomic::{AtomicUsize, Ordering};

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::http_analytics_loggers::delivery::Delivery;

use super::logger::OpikLogger;
use super::record::Payload;

// BatchQueue's flush hands the drained batch here, so a flush runs the same traces-then-spans send as async_send_batch.
pub(super) struct OpikBatch<'logger, D> {
    pub(super) logger: &'logger OpikLogger<D>,
}

impl<D> BatchDelivery<Payload> for OpikBatch<'_, D>
where
    D: Delivery + 'static,
{
    async fn deliver(
        &self,
        batch: &[Payload],
        settled: &AtomicUsize,
    ) -> Result<Vec<Payload>, BatchCancelled<Payload>> {
        self.logger.submit_entries(batch).await;
        settled.fetch_add(batch.len(), Ordering::SeqCst);
        Ok(Vec::new())
    }
}
