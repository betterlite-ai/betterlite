use std::sync::Arc;

use async_trait::async_trait;
use axum::http::request::Parts;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use futures_util::stream::BoxStream;

use crate::keepalive_interval::PingSetting;
use crate::stream_chunks::StreamItem;
use crate::stream_chunks::restamp::RouterCheck;
use crate::stream_keepalive::KeepaliveRouter;

pub type ItemStream = BoxStream<'static, Result<StreamItem, StreamFailure>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Flag(pub(super) bool);

// getattr(e, "param", "None") as a JSON field: the attribute is absent, present and null, or text
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureParam {
    Missing,
    Null,
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Http,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct StreamFailure {
    pub message: String,
    pub error_type: Option<String>,
    pub param: FailureParam,
    pub status_code: Option<u16>,
    pub kind: FailureKind,
}

pub struct StreamSource {
    pub items: ItemStream,
    pub hidden: JsonObject,
    pub snapshot: Option<Arc<dyn StreamSnapshot>>,
}

pub trait StreamSnapshot: Send + Sync {
    fn collected(&self) -> CollectedStream;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectedStream {
    pub chunks: Vec<JsonObject>,
    pub messages: Option<Vec<Json>>,
    pub model: Option<String>,
}

#[async_trait]
pub trait StreamHooks<Key>: Send + Sync
where
    Key: Sync,
{
    fn needs_iterator_wrap(&self) -> bool;

    fn needs_per_chunk_hook(&self) -> bool;

    fn wrap_iterator(&self, items: ItemStream, key: &Key, request_data: &JsonObject) -> ItemStream;

    async fn chunk_hook(
        &self,
        item: StreamItem,
        key: &Key,
        request_data: &JsonObject,
        str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure>;

    async fn failure_hook(
        &self,
        key: &Key,
        failure: &StreamFailure,
        request_data: &JsonObject,
    ) -> Result<(), StreamFailure>;

    fn fire_deferred_stream_logging(&self, request_data: &JsonObject);
}

pub trait StreamErrorState: Send {
    fn observe_chunk(&mut self, item: &StreamItem);

    fn mark_emitted(&mut self, frame: String) -> String;

    fn format_failure(&mut self, failure: &StreamFailure) -> Option<String>;
}

pub trait ErrorStateSource: Send + Sync {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send>;
}

pub struct CleanupOutcome<Key> {
    pub request: Option<Parts>,
    pub request_data: JsonObject,
    pub key: Key,
    pub response: Option<Arc<dyn StreamSnapshot>>,
    pub stream_completed: bool,
    pub client_disconnected: bool,
}

pub trait StreamCleanup<Key>: Send + Sync {
    fn finalize(&self, outcome: CleanupOutcome<Key>);
}

pub struct SseDataGenerator<Key>
where
    Key: Sync,
{
    pub hooks: Arc<dyn StreamHooks<Key>>,
    pub error_state: Arc<dyn ErrorStateSource>,
    pub cleanup: Arc<dyn StreamCleanup<Key>>,
    pub router: Option<Arc<dyn KeepaliveRouter>>,
    pub global_keepalive: Option<PingSetting>,
    pub expose_fallback_errors: bool,
    pub is_model_router: RouterCheck,
}
