use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use std::fmt::Debug;

use super::GcsBucketLogger;
use super::QueuedItem;
use super::single_item_behaviors::send_single_log_item;

pub async fn send_individual_logs<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    items: &[QueuedItem],
) where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    for item in items {
        send_single_log_item(logger, item).await;
    }
}

mod tests;
