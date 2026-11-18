use std::sync::{Arc, Mutex};

use betterlite_core::api::responses_stream_hooks::ResponsesStreamHooks;
use betterlite_core::api::responses_stream_hooks::ports::ChunkDeploymentHook;
use betterlite_core::api::responses_stream_primitives::state::{
    ResponsesStreamBaseState, ResponsesStreamHiddenParams,
};
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::ResponsesAPIStreamingResponse;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;
use tokio_util::task::TaskTracker;

use super::ports;

#[derive(Clone, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each field selects an independent oracle scenario"
)]
pub(crate) struct Row {
    pub event: ResponsesAPIStreamingResponse,
    pub kwargs: Option<IndexMap<String, Json>>,
    pub preset: Option<String>,
    pub request: IndexMap<String, Json>,
    pub details: IndexMap<String, Json>,
    pub processed: IndexMap<String, Json>,
    pub raw: IndexMap<String, String>,
    pub call_type: Option<String>,
    pub logging_type: Option<String>,
    pub actions: Vec<String>,
    pub eligible: bool,
    pub broken: bool,
    pub deferred: bool,
    pub cached: bool,
    pub yielded: bool,
}

pub(crate) struct Probe {
    pub row: Row,
    pub records: Mutex<Vec<Value>>,
    pub clock: DateTime<FixedOffset>,
}

impl SuccessClock for Probe {
    fn now(&self) -> DateTime<FixedOffset> {
        self.clock
    }
}

pub(crate) fn context<'context>(
    probe: &Arc<Probe>,
    redactor: &'context Redactor,
) -> ResponsesStreamHooks<'context> {
    ResponsesStreamHooks {
        cache: Some(Arc::<Probe>::clone(probe)),
        cache_handler: Some(Arc::<Probe>::clone(probe)),
        success: Some(Arc::<Probe>::clone(probe)),
        failure: Some(Arc::<Probe>::clone(probe)),
        metadata: Some(Arc::<Probe>::clone(probe)),
        success_hook: Some(Arc::<Probe>::clone(probe)),
        chunk_hooks: probe
            .row
            .actions
            .iter()
            .map(|action| -> Arc<dyn ChunkDeploymentHook> {
                Arc::new(ports::Chunk {
                    action: action.clone(),
                    probe: Arc::clone(probe),
                })
            })
            .collect(),
        clock: Arc::<Probe>::clone(probe),
        redactor,
    }
}

pub(crate) fn state(probe: &Probe) -> ResponsesStreamBaseState {
    ResponsesStreamBaseState {
        model: "gpt-4o".to_owned(),
        custom_llm_provider: Some("openai".to_owned()),
        litellm_metadata: None,
        request_data: probe.row.request.clone(),
        call_type: probe.row.call_type.clone(),
        finished: false,
        failure_handled: false,
        yielded_first_chunk: probe.row.yielded,
        output_started: false,
        completed_response: Some(probe.row.event.clone()),
        completed_response_cached: false,
        completed_response_logged: false,
        completed_response_cache_hit: probe.row.cached.then_some(true),
        persist_before_logging: !probe.row.cached,
        generated_content: "text".to_owned(),
        generated_tool_arguments: "tool".to_owned(),
        start_time: probe.clock,
        created_timestamp: 0.0,
        max_streaming_duration_seconds: None,
        hidden_params: ResponsesStreamHiddenParams {
            model_id: None,
            api_base: None,
            custom_llm_provider: None,
            additional_headers: probe.row.processed.clone(),
        },
        raw_response_headers: probe.row.raw.clone(),
        pending_tasks: TaskTracker::new(),
    }
}
