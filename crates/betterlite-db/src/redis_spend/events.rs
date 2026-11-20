use super::keys::SpendBufferQueue;

// _emit_new_item_added_to_redis_buffer_event's call_type, the label every queue-size gauge carries.
pub const REDIS_BUFFER_EVENT_CALL_TYPE: &str = "_emit_new_item_added_to_queue_event";

// The source emits with duration zero, the queue's service as the gauge label and the post-push list length as its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisBufferEvent {
    pub queue: SpendBufferQueue,
    pub queue_size: i64,
}

// service_logger_obj.async_service_success_hook, behind a port so the logging sink and its delivery stay outside this crate.
pub trait RedisBufferEventSink: Send + Sync {
    fn emit(&self, event: RedisBufferEvent);

    // _emit_new_item_added_to_redis_buffer_event: the queue names its gauge label and the post-push length is the value.
    fn emit_queue_size(&self, queue: SpendBufferQueue, queue_size: i64) {
        self.emit(RedisBufferEvent { queue, queue_size });
    }
}

#[derive(Debug, Default)]
pub struct NoRedisBufferEventSink;

impl RedisBufferEventSink for NoRedisBufferEventSink {
    fn emit(&self, _event: RedisBufferEvent) {}
}
