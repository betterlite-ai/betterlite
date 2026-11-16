use std::sync::Arc;

use betterlite_anthropic::stream_state::state::IdSource;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink};
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::streamed_response_assembly::cost::StreamCostCalculator;

// What a streamed response's assembly reads besides the chunks: LiteLLM's logging object, token counter, id source and clock.
pub struct StreamServices<'services> {
    pub costs: &'services dyn StreamCostCalculator,
    pub counter: &'services dyn UsageTokenCounter,
    pub redactor: Arc<Redactor>,
    pub ids: &'services dyn IdSource,
    pub now: i64,
}

pub(crate) struct SilentLogger;

impl LoggerSink for SilentLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        true
    }

    fn info(&self, _payload: &Payload) -> bool {
        true
    }

    fn error(&self, _payload: &Payload) -> bool {
        true
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

pub(crate) struct SilentConsole;

impl ConsoleSink for SilentConsole {
    fn print(&self, _payload: &Payload) -> bool {
        true
    }
}

mod tests;
