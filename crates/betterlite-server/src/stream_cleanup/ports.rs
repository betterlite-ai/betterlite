use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

pub type CleanupFuture<'call, Output> = Pin<Box<dyn Future<Output = Output> + Send + 'call>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CleanupError {
    #[error("{0}")]
    Port(String),
}

pub trait RequestLogging: Send + Sync {
    fn call_details(&self, request_data: &JsonObject) -> Option<JsonObject>;

    fn edit_call_details(&self, request_data: &JsonObject, edit: fn(&mut JsonObject));

    fn deferred_logging_armed(&self, request_data: &JsonObject) -> bool;

    fn fire_deferred_logging(&self, request_data: &JsonObject);

    fn baseline_cache_active(&self, request_data: &JsonObject) -> bool;

    fn invalidate_baseline<'call>(
        &'call self,
        request_data: &'call JsonObject,
        reason: &'call str,
        completed: bool,
    ) -> CleanupFuture<'call, Result<(), CleanupError>>;

    fn assemble_partial<'call>(
        &'call self,
        request_data: &'call JsonObject,
        chunks: &'call [JsonObject],
        messages: Option<&'call [Json]>,
    ) -> CleanupFuture<'call, Result<Option<JsonObject>, CleanupError>>;

    fn dispatch_success<'call>(
        &'call self,
        request_data: &'call JsonObject,
        response: JsonObject,
    ) -> CleanupFuture<'call, Result<(), CleanupError>>;
}

pub trait SlotRelease<Key>: Send + Sync {
    fn release_on_disconnect<'call>(
        &'call self,
        key: &'call Key,
    ) -> CleanupFuture<'call, Result<(), CleanupError>>;
}
