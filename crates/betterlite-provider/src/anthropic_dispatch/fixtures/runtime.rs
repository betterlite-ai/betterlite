use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink, PrePostEnv};
use betterlite_logging::logging_state::LoggingState;
use betterlite_redact::secret::Redactor;
use chrono::DateTime;

use super::FixtureError;
use crate::anthropic_dispatch::gateway::logging;

struct Discard;
impl LogSink for Discard {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) fn logging(redactor: &Redactor) -> Result<(LoggingState, PrePostEnv<'_>), FixtureError> {
    logging::state(
        &Discard,
        redactor,
        "claude",
        "call",
        DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap_or_default()
            .fixed_offset(),
    )
    .map_err(FixtureError::from)
}
