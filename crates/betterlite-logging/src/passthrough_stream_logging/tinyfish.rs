use async_trait::async_trait;
use chrono::{DateTime, FixedOffset};

use super::object::StreamLoggingObject;
use super::types::{HandlerError, HandlerOutcome};

pub struct TinyfishCollectedChunks<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub url_route: &'call str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub lines: &'call [String],
}

#[async_trait]
pub trait TinyfishStreamBilling: Send + Sync {
    fn sse_poller_spawned(&self, logging_obj: &StreamLoggingObject) -> bool;
    fn run_id_from_sse_frames(&self, frames: &[u8]) -> Option<String>;
    fn start_sse_run_billing(
        &self,
        run_id: &str,
        logging_obj: &StreamLoggingObject,
        start_time: DateTime<FixedOffset>,
    );
    // ast-grep-ignore: no-long-names
    async fn handle_logging_tinyfish_collected_chunks(
        &self,
        call: &TinyfishCollectedChunks<'_>,
    ) -> Result<HandlerOutcome, HandlerError>;
}
