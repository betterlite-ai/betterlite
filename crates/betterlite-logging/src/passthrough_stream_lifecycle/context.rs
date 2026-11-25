use std::sync::Arc;

use chrono::{DateTime, FixedOffset};

use crate::passthrough_logging_state::logger::PassThroughEndpointLogging;

// LiteLLM's frozen PassThroughStreamContext: the three values a failed stream needs to log its partial usage
#[derive(Clone)]
pub struct PassThroughStreamContext {
    pub success_handler: Arc<PassThroughEndpointLogging>,
    pub url_route: String,
    pub start_time: DateTime<FixedOffset>,
}

mod tests;
