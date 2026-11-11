use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, restore_correlation_context,
};
use betterlite_models::decisions::Json;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatStream {
    pub model_response: Json,
    pub json_mode: bool,
    pub converted_from_mock: bool,
}

// The three shapes a call can return; the sync path never stamps correlation ids, so only async code asks
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallOutcome {
    Response(Json),
    ChatStream(ChatStream),
    ResponsesStream(Json),
}

impl CallOutcome {
    #[must_use]
    pub const fn is_streaming_response(&self) -> bool {
        match self {
            Self::ChatStream(_) => true,
            Self::Response(_) | Self::ResponsesStream(_) => false,
        }
    }

    #[must_use]
    pub const fn is_converted_stream(&self) -> bool {
        match self {
            Self::ChatStream(_) | Self::ResponsesStream(_) => true,
            Self::Response(_) => false,
        }
    }
}

// LiteLLM calls this only when the logging object has the method; a Rust state always does
pub trait CorrelationRestore {
    fn restore_correlation_context(&self, context: &mut CorrelationContext);
}

impl CorrelationRestore for LoggingState {
    fn restore_correlation_context(&self, context: &mut CorrelationContext) {
        restore_correlation_context(self, context);
    }
}
