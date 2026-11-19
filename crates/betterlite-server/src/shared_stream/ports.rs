use crate::stream_chunks::StreamItem;

// LiteLLM's release_budget_reservation_on_cancel: the production impl runs it on the server's task tracker, since Drop calls it
pub trait ReservationRelease<Key>: Send + Sync {
    fn release_on_cancel(&self, key: &Key);
}

// LiteLLM's _process_chunk_with_cost_injection, passed only when include_cost_in_streaming_usage is on
pub trait CostInjection: Send + Sync {
    fn inject(&self, item: StreamItem, model_name: &str) -> StreamItem;
}

// LiteLLM's _withheld_provider_output reads this from the response object
pub trait BufferedOutput: Send + Sync {
    fn has_buffered_provider_output(&self) -> bool;
}
