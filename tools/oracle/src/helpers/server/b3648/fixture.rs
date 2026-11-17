use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::Redactor;
use serde_json::json;

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

// The same shape the pass-through route builds: one request's logging state, owned by a dispatcher that does nothing.
pub(super) fn logging_object() -> Result<StreamLoggingObject, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_value(json!({
        "model": "fixture-model",
        "call_type": "pass_through_endpoint",
        "start_time": "2024-01-01T00:00:00+00:00",
        "litellm_call_id": "call-1",
        "function_id": "function-1",
        "stream": false,
    }))
    .or_value_error()?;
    let redactor = Redactor::new(16).or_value_error()?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        init,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .or_value_error()?;
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    })
}
