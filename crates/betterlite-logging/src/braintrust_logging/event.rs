use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackPath {
    Sync,
    Async,
}

#[derive(Debug, Clone, Copy)]
pub enum ResponseObject<'response> {
    Model(&'response Json),
    TextCompletion(&'response Json),
    Embedding(&'response Json),
    Image(&'response Json),
}

#[derive(Debug, Clone, Copy)]
pub struct SuccessEvent<'call> {
    pub call_id: Option<&'call Json>,
    pub messages: Option<&'call Json>,
    pub standard_logging_object: Option<&'call Json>,
    pub metadata: Option<&'call Json>,
    pub response_cost: Option<&'call Json>,
    pub call_type: Option<&'call str>,
    pub response: Option<ResponseObject<'call>>,
    pub api_call_start_time: Option<DateTime<Utc>>,
    pub completion_start_time: Option<DateTime<Utc>>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}
