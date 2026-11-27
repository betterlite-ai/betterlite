use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;

const INIT: &str = r#"{"model":"jamba-1.5","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-b10159","function_id":"fn-b10159","litellm_trace_id":"trace-b10159"}"#;
const NOW: &str = "2026-01-01T00:00:00+00:00";

// The post_call log the transforms write through, with the record dropped: no case projects it.
pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Logging {
    redactor: Redactor,
    sink: Quiet,
}

impl Logging {
    pub(super) fn build() -> Result<Self, Box<HelperError>> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_config_error()?;
        Ok(Self {
            redactor,
            sink: Quiet,
        })
    }

    pub(super) fn setup(&self) -> Result<(LoggingState, PrePostEnv<'_>), Box<HelperError>> {
        let mut context = CorrelationContext::default();
        let init = serde_json::from_str::<LoggingInit>(INIT).or_config_error()?;
        let setup = CorrelationSetup {
            settings: CorrelationSettings::default(),
            context: &mut context,
        };
        let state = LoggingState::new(
            init,
            setup,
            &self.redactor,
            || String::from("b10159"),
            Duration::from_secs(0),
        )
        .or_config_error()?;
        let now = DateTime::<FixedOffset>::parse_from_rfc3339(NOW).or_config_error()?;
        let env = PrePostEnv {
            redactor: &self.redactor,
            log: &self.sink,
            now,
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                key_info: false,
                content: false,
            },
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
        };
        Ok((state, env))
    }
}

pub(super) fn header_map(values: &IndexMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let pairs = values.iter().map(|(name, value)| {
        let header_name = HeaderName::try_from(name.as_str()).or_config_error()?;
        let header_value = HeaderValue::try_from(value.as_str()).or_config_error()?;
        Ok::<_, Box<HelperError>>((header_name, header_value))
    });
    pairs.collect()
}
