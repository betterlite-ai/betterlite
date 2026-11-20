use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

// A real logging state for the constructor to record its msg_ id on; serde fills every field but these five.
pub(super) fn logging_state(
    enabled: bool,
    model: &str,
) -> Result<Option<LoggingState>, Box<HelperError>> {
    if !enabled {
        return Ok(None);
    }
    let init = serde_json::from_value::<LoggingInit>(serde_json::json!({
        "model": model,
        "call_type": "anthropic_messages",
        "start_time": "2026-01-01T00:00:00+00:00",
        "litellm_call_id": "call-1",
        "function_id": "fn-1",
    }))
    .or_value_error()?;
    let mut context = CorrelationContext::default();
    let state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .or_value_error()?;
    Ok(Some(state))
}
