mod steps;

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;

use async_trait::async_trait;
use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_core::api::messages_chat_transform::adapter::AnthropicAdapter;
use betterlite_core::api::messages_chat_transform::stream::{StreamOptions, StreamOutput};
use betterlite_core::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, PolyfillResult, UsageIteration,
};
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_core::api::messages_stream_split::{AsyncChunkSource, ChunkSource};
use betterlite_models::decisions;
use betterlite_streaming::provider_chunk_decode::chunk::ProviderChunk;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamChunk};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;
use super::wrapper_stream::{self, WireCall};
use super::{ChatMessage, OracleLimits};
use crate::sdk_http::call_checks::validate_timeout;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    model: String,
    messages: Vec<ChatMessage>,
    api_key: String,
    timeout_seconds: u64,
    #[serde(default)]
    tool_name_mapping: IndexMap<String, String>,
    #[serde(default)]
    applied_edits: Vec<AppliedEdit>,
    #[serde(default)]
    compaction_block: Option<CompactionBlock>,
    #[serde(default)]
    iterations_usage: Option<Vec<UsageIteration>>,
    runs: Vec<RunSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    // LiteLLM's keyword defaults to true, so a case that omits it asks for the async view
    #[serde(default = "default_is_async")]
    is_async: bool,
    steps: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Next,
    Drain,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    runs: Vec<RunOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    is_async: bool,
    steps: Vec<StepOutcome>,
}

// One call of the factory's result: the frames it wrote, whether the iteration ended, and the view's metadata
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepOutcome {
    frames: Vec<String>,
    end: bool,
    view: Option<ViewOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewOutcome {
    model: String,
    messages: Option<Vec<decisions::Json>>,
    chunks: Option<Vec<String>>,
}

const fn default_is_async() -> bool {
    true
}

// The mock's SSE payloads decode through B005's chunk reader, and the source keeps what the view reports
struct Prefetched {
    payloads: VecDeque<Vec<u8>>,
    chunks: Vec<StreamChunk>,
    messages: Vec<decisions::Json>,
}

#[async_trait]
impl AsyncChunkSource for Prefetched {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }

    fn messages(&self) -> Option<&[decisions::Json]> {
        Some(&self.messages)
    }
}

impl ChunkSource for Prefetched {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        Some(&self.chunks)
    }

    fn messages(&self) -> Option<&[decisions::Json]> {
        Some(&self.messages)
    }
}

impl Prefetched {
    fn pull(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        let Some(payload) = self.payloads.pop_front() else {
            return Ok(None);
        };
        let value: JsonValue =
            serde_json::from_slice(&payload).map_err(|error| StreamFailure::Unclassified {
                text: error.to_string(),
            })?;
        let chunk = ProviderChunk::Json(Box::new(value))
            .openai_chunk()
            .map_err(|error| StreamFailure::Unclassified {
                text: error.to_string(),
            })?;
        self.chunks.push(chunk.clone());
        Ok(Some(chunk))
    }
}

impl Call {
    fn polyfill_result(&self) -> Option<PolyfillResult> {
        if self.applied_edits.is_empty()
            && self.compaction_block.is_none()
            && self.iterations_usage.is_none()
        {
            return None;
        }
        Some(PolyfillResult {
            messages: Vec::new(),
            system: None,
            applied_edits: self.applied_edits.clone(),
            compaction_block: self.compaction_block.clone(),
            iterations_usage: self.iterations_usage.clone(),
        })
    }
}

impl WireCall for Call {
    fn model(&self) -> &str {
        &self.model
    }

    fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    fn api_key(&self) -> &str {
        &self.api_key
    }

    fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }
}

impl CallKind for Call {
    // Only the async view can be closed; the sync iterator is the wrapper's own generator over bytes
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        validate_timeout(self.timeout_seconds, limits)?;
        for run in &self.runs {
            if !run.is_async && run.steps.contains(&Step::Close) {
                return Err(SdkHttpError::Invalid(
                    "only the async view can be closed".to_owned(),
                ));
            }
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        send(self, api_base, env)
    }
}

async fn send(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let prepared = wrapper_stream::prepare(call, api_base, env)?;
    let mut runs = Vec::with_capacity(call.runs.len());
    for spec in &call.runs {
        let payloads = wrapper_stream::prefetch(&prepared, call).await?;
        runs.push(run(call, spec, payloads).await?);
    }
    Ok(SdkResult::MessagesChatTransformStream {
        value: Box::new(Outcome { runs }),
    })
}

// The production factory picks the mode; a case drives whichever iterator the flag chose
async fn run(
    call: &Call,
    spec: &RunSpec,
    payloads: VecDeque<Vec<u8>>,
) -> Result<RunOutcome, SdkHttpError> {
    let source = Prefetched {
        payloads,
        chunks: Vec::new(),
        messages: call.messages.iter().map(message_json).collect(),
    };
    let polyfill = call.polyfill_result();
    let mut options = StreamOptions::new(&call.model);
    options.tool_name_mapping = Some(&call.tool_name_mapping);
    options.polyfill_result = polyfill.as_ref();
    options.is_async = spec.is_async;
    let output = AnthropicAdapter::new()
        .translate_completion_output_params_streaming(source, options, UuidV4);
    match output {
        StreamOutput::Async(view) => {
            let pump = steps::Pump::Async(view);
            steps::run_steps(pump, &spec.steps).await
        }
        StreamOutput::Sync(frames) => {
            let pump = steps::Pump::Sync(frames);
            steps::run_steps(pump, &spec.steps).await
        }
    }
}

fn message_json(message: &ChatMessage) -> decisions::Json {
    decisions::Json::Object(IndexMap::from([
        (
            "role".to_owned(),
            decisions::Json::Text(message.role.clone()),
        ),
        (
            "content".to_owned(),
            decisions::Json::Text(message.content.clone()),
        ),
    ]))
}
