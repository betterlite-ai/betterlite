use std::sync::{Arc, Mutex};

use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::CorrelationContext;

use super::logging::AsyncSuccessLog;

// The logging object LiteLLM threads through kwargs as litellm_logging_obj, plus the slot a deferred success log waits in
#[derive(Clone)]
pub struct LoggingHandle {
    pub state: Arc<Mutex<LoggingState>>,
    pub correlation: Arc<Mutex<CorrelationContext>>,
    pub deferred: Arc<Mutex<Option<AsyncSuccessLog>>>,
}

// A nested wrapper shares the handle it was handed; internal_call is the proxy's _is_litellm_internal_call contextvar
#[derive(Clone, Default)]
pub struct WrapperOptions {
    pub logging: Option<LoggingHandle>,
    pub internal_call: bool,
}

// The per-request callbacks function_setup reads reach it through this, so a name only counts when the catalog knows it
pub trait KnownCallback: Send + Sync {
    fn is_known(&self, name: &str) -> bool;
}
