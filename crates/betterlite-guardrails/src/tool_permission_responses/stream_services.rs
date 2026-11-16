use std::sync::Arc;

use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;
use betterlite_streaming::stream_usage_assembly::UsageTokenCounter;
use betterlite_streaming::streamed_response_assembly::cost::StreamCostCalculator;

// What a streamed response's assembly reads besides the chunks: LiteLLM's logging object, token counter and clock.
pub struct StreamServices<'services> {
    pub costs: &'services dyn StreamCostCalculator,
    pub counter: &'services dyn UsageTokenCounter,
    pub redactor: Arc<Redactor>,
    pub now: i64,
}

impl StreamServices<'_> {
    // The builder reports through VerboseHooks; a guardrail adds no verbose output of its own.
    pub(crate) fn hooks(&self) -> VerboseHooks<SilentLogger, SilentConsole> {
        VerboseHooks {
            logger: SilentLogger,
            console: SilentConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::clone(&self.redactor),
        }
    }
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
