use std::sync::Arc;

use async_trait::async_trait;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::error::LoggerError;
use crate::callback_logger::event::LoggerEvent;
use crate::langfuse_observations::attributes::keys::{
    OBSERVATION_INPUT, OBSERVATION_OUTPUT, TRACE_NAME, TRACE_SESSION_ID, TRACE_TAGS, TRACE_USER_ID,
};
use crate::logging_pre_post::error::PrePostError;
use crate::logging_pre_post::hooks::{ApiCallLogger, PostCallEvent, PreCallEvent};
use crate::otel::content_projections::{
    OutputResponse, RelayedChunk, StreamOutputAssembler, request_input, response_output,
};
use crate::otel::langfuse_root::relay::StampingStream;
use crate::otel::logger::v2::OpenTelemetryV2;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::trace_controls::{TraceControls, caller_trace_controls};
use crate::otel::model::utils::coercion::OtelModelValue;

pub mod relay;

// the request's root SERVER span: Langfuse reads a trace's name, user, session, tags and content off it
pub trait RootSpanSink: Send + Sync {
    fn is_recording(&self) -> bool;

    fn set_attribute(&self, key: &str, value: String);
}

// the live root span the server layer owns; B3847's slot keeps only its SpanContext, which cannot record
pub trait RootSpanSource: Send + Sync {
    fn request_root_span(&self) -> Option<Arc<dyn RootSpanSink>>;
}

pub struct LangfuseOtel {
    pub inner: OpenTelemetryV2,
    pub roots: Arc<dyn RootSpanSource>,
}

// the wrapped logger keeps every other CallbackLogger method at its trait default, so only the overrides forward
#[async_trait]
impl CallbackLogger for LangfuseOtel {
    fn callback_name(&self) -> &str {
        self.inner.callback_name()
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.inner.async_log_success_event(event).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.inner.async_log_failure_event(event).await
    }
}

impl ApiCallLogger for LangfuseOtel {
    // the root is still recording while the LLM call starts, so the trace controls land here, before the wrapped hook
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        if let Some(root) = self.roots.request_root_span()
            && root.is_recording()
        {
            let kwargs: OtelModelValue = serde_json::from_value(
                serde_json::to_value(event.details).map_err(|error| callback_failed(&error))?,
            )
            .map_err(|error| callback_failed(&error))?;
            for (key, value) in trace_attributes(&caller_trace_controls(&kwargs)) {
                root.set_attribute(key, value);
            }
        }
        self.inner.log_pre_api_call(event)
    }

    // the post-call has nothing to do here: the boundary span closes on the success or failure callback
    fn log_post_api_call(&self, _event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

pub struct LangfuseContentOtel {
    pub inner: LangfuseOtel,
    pub stream_assembler: Arc<dyn StreamOutputAssembler>,
}

#[async_trait]
impl CallbackLogger for LangfuseContentOtel {
    fn callback_name(&self) -> &str {
        self.inner.callback_name()
    }

    async fn async_log_success_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.inner.async_log_success_event(event).await
    }

    async fn async_log_failure_event(&self, event: Arc<LoggerEvent>) -> Result<(), LoggerError> {
        self.inner.async_log_failure_event(event).await
    }
}

impl ApiCallLogger for LangfuseContentOtel {
    fn log_pre_api_call(&self, event: &PreCallEvent<'_>) -> Result<(), PrePostError> {
        self.inner.log_pre_api_call(event)
    }

    // the post-call has nothing to do here: the boundary span closes on the success or failure callback
    fn log_post_api_call(&self, _event: &PostCallEvent<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

impl LangfuseContentOtel {
    // ast-grep-ignore: no-one-line-functions - LangfuseContentOpenTelemetryV2.async_post_call_success_hook, called by B8910
    pub fn async_post_call_success_hook(
        &self,
        data: &MetadataMap,
        _user_api_key_dict: &MetadataMap,
        response: &OutputResponse<'_>,
    ) {
        self.stamp_root_io(data, || response_output(response));
    }

    // litellm.integrations.otel.langfuse_logger.LangfuseContentOpenTelemetryV2.async_post_call_streaming_iterator_hook
    #[must_use]
    // ast-grep-ignore: no-long-names - B8910 calls this hook by the LiteLLM name
    pub fn async_post_call_streaming_iterator_hook<'stream, S>(
        &'stream self,
        _user_api_key_dict: &MetadataMap,
        response: S,
        request_data: &'stream MetadataMap,
    ) -> StampingStream<'stream, S>
    where
        S: futures_util::Stream<Item = RelayedChunk> + Send,
    {
        StampingStream {
            inner: Box::pin(response),
            logger: self,
            request_data,
            relayed: Vec::new(),
            finished: false,
        }
    }

    // litellm.integrations.otel.langfuse_logger.LangfuseContentOpenTelemetryV2._stamp_root_io: the output first, so a root missing its input keeps the output
    fn stamp_root_io<F>(&self, data: &MetadataMap, render_output: F)
    where
        F: FnOnce() -> Option<String>,
    {
        let Some(root) = self.inner.roots.request_root_span() else {
            return;
        };
        if !root.is_recording() {
            return;
        }
        let Some(output) = render_output() else {
            return;
        };
        root.set_attribute(OBSERVATION_OUTPUT, output);
        if let Some(input) = request_input(data) {
            root.set_attribute(OBSERVATION_INPUT, input);
        }
    }
}

// LangfuseMapper.trace_attributes: the caller's controls, with empty ones omitted
fn trace_attributes(controls: &TraceControls) -> Vec<(&'static str, String)> {
    let mut attributes = Vec::new();
    for (key, value) in [
        (TRACE_NAME, controls.name.as_deref()),
        (TRACE_USER_ID, controls.user_id.as_deref()),
        (TRACE_SESSION_ID, controls.session_id.as_deref()),
    ] {
        if let Some(text) = value.filter(|text| !text.is_empty()) {
            attributes.push((key, text.to_owned()));
        }
    }
    // the sink carries strings only, so the tag list travels as the JSON text the mapper built
    if !controls.tags.is_empty()
        && let Ok(tags) = serde_json::to_string(&controls.tags)
    {
        attributes.push((TRACE_TAGS, tags));
    }
    attributes
}

fn callback_failed(error: &serde_json::Error) -> PrePostError {
    PrePostError::CallbackFailed {
        message: error.to_string(),
    }
}

mod tests;
