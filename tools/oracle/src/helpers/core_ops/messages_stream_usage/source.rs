use std::fmt::Display;
use std::time::Duration;

use betterlite_core::api::messages_stream_split::ChunkSource;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use indexmap::IndexMap;

// The wrapper never pulls a chunk in these steps, so the stream only forwards what it was handed.
pub(super) struct ScriptedSource {
    pub(super) chunks_attr: Option<Vec<StreamChunk>>,
    pub(super) messages_attr: Option<Vec<Json>>,
}

impl ChunkSource for ScriptedSource {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        Ok(None)
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks_attr.as_deref()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.messages_attr.as_deref()
    }
}

fn helper_failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "StateError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

// The constructor records its id on a real LoggingState, so the snapshot can compare the two.
pub(super) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let mut context = CorrelationContext::default();
    LoggingState::new(
        LoggingInit {
            model: "fixture".to_owned(),
            call_type: "fixture".to_owned(),
            start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
                .map_err(helper_failure)?,
            litellm_call_id: "call-1".to_owned(),
            function_id: "fn-1".to_owned(),
            messages: None,
            stream: None,
            litellm_trace_id: Some("trace-1".to_owned()),
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags::default(),
            callback_policy: CallbackPolicy::default(),
        },
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(helper_failure)?,
        || "generated-trace".to_owned(),
        Duration::default,
    )
    .map_err(helper_failure)
}
