// ServiceTypes.IN_MEMORY_SPEND_UPDATE_QUEUE and ServiceTypes.IN_MEMORY_DAILY_SPEND_UPDATE_QUEUE of litellm/types/services.py.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendQueueService {
    InMemorySpendUpdateQueue,
    InMemoryDailySpendUpdateQueue,
}

impl SpendQueueService {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InMemorySpendUpdateQueue => "in_memory_spend_update_queue",
            Self::InMemoryDailySpendUpdateQueue => "in_memory_daily_spend_update_queue",
        }
    }
}

// _emit_new_item_added_to_queue_event: service, call_type _emit_new_item_added_to_queue_event, duration zero and the queue size as the gauge value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendQueueEvent {
    pub service: SpendQueueService,
    pub queue_size: usize,
}

// service_logger_obj.async_service_success_hook, behind a port so the service sink and its delivery stay outside this crate.
pub trait SpendQueueEventSink: Send + Sync {
    fn emit(&self, event: SpendQueueEvent);
}

#[derive(Debug, Default)]
pub struct NoSpendQueueEventSink;

impl SpendQueueEventSink for NoSpendQueueEventSink {
    fn emit(&self, _event: SpendQueueEvent) {}
}
