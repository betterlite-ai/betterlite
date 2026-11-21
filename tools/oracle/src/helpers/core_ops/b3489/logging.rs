use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::Redactor;

// the logging state every entry point fills, built from a fixed fixture so no clock or file is read
pub(super) fn logging_state(redactor: &Redactor) -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"gemini-2.5-flash","call_type":"generate_content","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .or_value_error()?;
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_value_error()
}
