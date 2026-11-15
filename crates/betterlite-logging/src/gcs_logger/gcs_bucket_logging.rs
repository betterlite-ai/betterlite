use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::prometheus_query::clock::Clock;

use self::event_enqueue_behaviors::enqueue;
use self::initialization_behaviors::GcsSettings;
use super::runtime::IdSource;

pub mod async_send_behaviors;
pub mod config_key_behaviors;
pub mod drain_batch_behaviors;
pub mod event_enqueue_behaviors;
pub mod flush_behaviors;
pub mod grouped_batch_behaviors;
pub mod grouping_behaviors;
pub mod individual_logs_behaviors;
pub mod initialization_behaviors;
pub mod naming_format_behaviors;
pub mod ndjson_behaviors;
pub mod object_naming_behaviors;
pub mod payload_retrieval_behaviors;
pub mod single_item_behaviors;

mod tests;

pub const CALLBACK_NAME: &str = "gcs_bucket";

// one queued event keeps the kwargs and the response object LiteLLM reads when it names and sends the item
#[derive(Clone, Debug)]
pub struct QueuedItem {
    pub payload: Arc<StandardLoggingPayload>,
    pub dynamic_params: Arc<Json>,
    pub metadata: Arc<IndexMap<String, Json>>,
    pub response_id: String,
}

pub struct GcsBucketLogger<Auth, Http> {
    pub base: GcsBucketBase<Auth, Http>,
    pub settings: GcsSettings,
    pub queue: Mutex<VecDeque<QueuedItem>>,
    pub last_flush_time: Mutex<DateTime<Utc>>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdSource>,
    pub stopped: CancellationToken,
}

pub struct GcsDeps {
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdSource>,
    pub tasks: Option<TaskTracker>,
    pub shutdown: CancellationToken,
}

#[async_trait]
impl<Auth, Http> CallbackLogger for GcsBucketLogger<Auth, Http>
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    fn callback_name(&self) -> &str {
        CALLBACK_NAME
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        enqueue(self, &event).await;
        Ok(())
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        enqueue(self, &event).await;
        Ok(())
    }
}
