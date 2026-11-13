use std::sync::Arc;

use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::http_analytics_loggers::delivery::Delivery;

use super::SqsLogger;

pub fn spawn_flush_loop<D>(logger: &Arc<SqsLogger<D>>, shutdown: CancellationToken)
where
    D: Delivery + 'static,
{
    let tasks = logger.tasks.clone();
    let flush = Arc::clone(logger);
    tasks.spawn(async move {
        run_flush_loop(&flush, shutdown).await;
    });
}

async fn run_flush_loop<D>(worker: &Arc<SqsLogger<D>>, shutdown: CancellationToken)
where
    D: Delivery + 'static,
{
    loop {
        let _woken = timeout(worker.queue.settings.flush_interval, shutdown.cancelled()).await;
        if shutdown.is_cancelled() {
            break;
        }
        worker.flush_queue().await;
    }
}
