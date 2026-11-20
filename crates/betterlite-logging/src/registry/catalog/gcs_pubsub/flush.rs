use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_models::decisions::Json;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::batch::delivery::{BatchCancelled, BatchDelivery};
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_pubsub::GcsPubSubLogger;
use crate::http_analytics_loggers::delivery::Delivery;

impl<Auth, D> BatchDelivery<Json> for GcsPubSubLogger<Auth, D>
where
    Auth: VertexAuth + 'static,
    D: Delivery + 'static,
{
    async fn deliver(
        &self,
        batch: &[Json],
        settled: &AtomicUsize,
    ) -> Result<Vec<Json>, BatchCancelled<Json>> {
        for message in batch {
            self.publish_message(message).await;
            settled.fetch_add(1, Ordering::SeqCst);
        }
        Ok(Vec::new())
    }
}

pub(crate) fn spawn_flush_loop<Auth, D>(
    logger: &Arc<GcsPubSubLogger<Auth, D>>,
    tasks: &TaskTracker,
    shutdown: CancellationToken,
) where
    Auth: VertexAuth + 'static,
    D: Delivery + 'static,
{
    let worker = Arc::clone(logger);
    tasks.spawn(async move {
        run_flush_loop(worker, shutdown).await;
    });
}

async fn run_flush_loop<Auth, D>(logger: Arc<GcsPubSubLogger<Auth, D>>, shutdown: CancellationToken)
where
    Auth: VertexAuth + 'static,
    D: Delivery + 'static,
{
    loop {
        let _woken = timeout(logger.queue.settings.flush_interval, shutdown.cancelled()).await;
        if shutdown.is_cancelled() {
            break;
        }
        logger.send_batch().await;
    }
}
