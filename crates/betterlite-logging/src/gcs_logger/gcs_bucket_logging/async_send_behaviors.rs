use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use std::fmt::Debug;

use super::GcsBucketLogger;
use super::drain_batch_behaviors::drain_queue_batch;
use super::grouped_batch_behaviors::send_grouped_batch;
use super::grouping_behaviors::group_items_by_config;
use super::individual_logs_behaviors::send_individual_logs;

pub async fn async_send_batch<Auth, Http>(logger: &GcsBucketLogger<Auth, Http>)
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let items = drain_queue_batch(logger);
    if items.is_empty() {
        return;
    }
    if logger.settings.use_batched_logging {
        match group_items_by_config(logger, &items) {
            Ok(grouped) => {
                for group in grouped.values() {
                    let (sent, errors) = send_grouped_batch(logger, group).await;
                    tracing::debug!(sent, errors, "GCS Bucket sent a grouped batch");
                }
            }
            Err(error) => tracing::error!(
                %error,
                "GCS Bucket logging error: the drained items have no valid dynamic config"
            ),
        }
    } else {
        send_individual_logs(logger, &items).await;
    }
}

mod tests;
