use std::collections::BTreeMap;

use crate::otel::legacy_policy::config::OpenTelemetryConfig;

mod tests;

pub const NO_CONTENT: &str = "NO_CONTENT";
pub const SPAN_ONLY: &str = "SPAN_ONLY";
pub const EVENT_ONLY: &str = "EVENT_ONLY";
pub const SPAN_AND_EVENT: &str = "SPAN_AND_EVENT";
pub const CAPTURE_MESSAGE_CONTENT_ENV: &str = "OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureMode {
    NoContent,
    SpanOnly,
    EventOnly,
    SpanAndEvent,
}

impl CaptureMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoContent => NO_CONTENT,
            Self::SpanOnly => SPAN_ONLY,
            Self::EventOnly => EVENT_ONLY,
            Self::SpanAndEvent => SPAN_AND_EVENT,
        }
    }

    #[must_use]
    pub const fn capture_in_span(self) -> bool {
        match self {
            Self::SpanOnly | Self::SpanAndEvent => true,
            Self::NoContent | Self::EventOnly => false,
        }
    }

    #[must_use]
    pub const fn capture_in_event(self) -> bool {
        match self {
            Self::EventOnly | Self::SpanAndEvent => true,
            Self::NoContent | Self::SpanOnly => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CapturePolicy {
    explicit: Option<CaptureMode>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Deserialize)]
pub struct LoggingFlags {
    pub turn_off_message_logging: bool,
    pub message_logging: bool,
}

impl CapturePolicy {
    #[must_use]
    pub fn from_init_state(config: &OpenTelemetryConfig, env: &BTreeMap<String, String>) -> Self {
        let from_config = config
            .capture_message_content
            .as_deref()
            .filter(|text| !text.is_empty());
        let from_env = env
            .get(CAPTURE_MESSAGE_CONTENT_ENV)
            .map(String::as_str)
            .filter(|text| !text.is_empty());
        Self {
            explicit: from_config.or(from_env).and_then(parse_mode),
        }
    }

    #[must_use]
    pub const fn resolve(&self, flags: LoggingFlags) -> CaptureMode {
        if flags.turn_off_message_logging {
            return CaptureMode::NoContent;
        }
        match self.explicit {
            Some(mode) => mode,
            None if flags.message_logging => CaptureMode::SpanAndEvent,
            None => CaptureMode::NoContent,
        }
    }
}

fn parse_mode(text: &str) -> Option<CaptureMode> {
    let normalized = text.to_uppercase();
    match normalized.as_str() {
        "TRUE" | "1" | EVENT_ONLY => Some(CaptureMode::EventOnly),
        "FALSE" | "0" | NO_CONTENT => Some(CaptureMode::NoContent),
        SPAN_ONLY => Some(CaptureMode::SpanOnly),
        SPAN_AND_EVENT => Some(CaptureMode::SpanAndEvent),
        _ => None,
    }
}
