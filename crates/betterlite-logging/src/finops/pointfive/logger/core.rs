use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use tokio::sync::Mutex as AsyncMutex;

use super::client::UploadClientSource;
use super::error::PointFiveError;
use super::pending::PendingQueue;
use crate::batch::logger::DEFAULT_MAX_QUEUE_SIZE;
use crate::finops::pointfive::upload::PointFiveUploadClient;

pub struct PointFiveCore {
    pub params: PointFiveInitParams,
    pub given_upload_client: Option<PointFiveUploadClient>,
    pub source: Arc<dyn UploadClientSource>,
    pub queue: PendingQueue,
    pub flush_lock: AsyncMutex<()>,
    pub flushing: AtomicBool,
    pub global_message_redaction: bool,
    pub max_queue_size: usize,
}

impl PointFiveCore {
    #[must_use]
    pub fn new(
        params: PointFiveInitParams,
        given_upload_client: Option<PointFiveUploadClient>,
        source: Arc<dyn UploadClientSource>,
        global_message_redaction: bool,
    ) -> Self {
        Self {
            params,
            given_upload_client,
            source,
            queue: PendingQueue::default(),
            flush_lock: AsyncMutex::new(()),
            flushing: AtomicBool::new(false),
            global_message_redaction,
            max_queue_size: DEFAULT_MAX_QUEUE_SIZE,
        }
    }

    // upload_client: built from the key and url configured right now, so a ui edit needs no restart
    pub fn upload_client(&self) -> Result<PointFiveUploadClient, PointFiveError> {
        if let Some(client) = &self.given_upload_client {
            return Ok(client.clone());
        }
        self.source.upload_client(&self.params)
    }

    pub fn drop_overflow(&self) {
        if self.flushing.load(Ordering::SeqCst) {
            return;
        }
        self.trim_queue();
    }

    pub(super) fn trim_queue(&self) {
        let dropped = self.queue.drop_overflow(self.max_queue_size);
        if dropped > 0 {
            tracing::warn!(
                max_queue_size = self.max_queue_size,
                dropped,
                "pointfive: queue over the cap, dropped the oldest records"
            );
        }
    }
}
