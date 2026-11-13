use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use async_trait::async_trait;
use betterlite_a2a::client::records::compat::SendMessageRequest;
use betterlite_a2a::streaming_iterator::{
    A2AStreamingIterator, DEFAULT_MAX_RETAINED_BYTES, StreamSetup, StreamSuccessCall,
    StreamSuccessDispatch, StreamedChunk,
};
use betterlite_core::agents::json::AgentJson;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::Stream;
use indexmap::IndexMap;
use serde::Deserialize;

use super::state::{details_value, state};
use super::tokenizer::Cl100kCounter;
use crate::compare::helper::HelperError;
use crate::helpers::a2a::b4810::helper_error;

#[derive(Deserialize)]
pub(crate) struct IterateInstance {
    request: SendMessageRequest,
    logging: IndexMap<String, Json>,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
    #[serde(default)]
    chunks: Vec<AgentJson>,
    #[serde(default)]
    failure: bool,
}

#[derive(Clone)]
struct WireChunk(AgentJson);

impl StreamedChunk for WireChunk {
    fn wire(&self) -> Result<AgentJson, serde_json::Error> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Copy, Debug)]
struct StreamFailure;

struct ScriptedStream {
    items: VecDeque<Result<WireChunk, StreamFailure>>,
}

impl Stream for ScriptedStream {
    type Item = Result<WireChunk, StreamFailure>;

    fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.get_mut().items.pop_front())
    }
}

#[derive(Default)]
struct Recorder {
    calls: Arc<Mutex<Vec<StreamSuccessCall>>>,
}

#[async_trait]
impl StreamSuccessDispatch for Recorder {
    async fn dispatch(&mut self, _logging: &mut LoggingState, call: StreamSuccessCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn dispatch_value(recorder: &Recorder) -> Option<AgentJson> {
    recorder
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .first()
        .map(|call| {
            serde_json::json!({
                "result": call.result,
                "start_time": call.start_time.to_rfc3339(),
                "end_time": call.end_time.to_rfc3339(),
                "cache_hit": call.cache_hit,
                "prefer_async_handlers": call.prefer_async_handlers,
            })
        })
}

fn scripted(instance: &IterateInstance) -> ScriptedStream {
    let mut items: VecDeque<Result<WireChunk, StreamFailure>> = instance
        .chunks
        .iter()
        .cloned()
        .map(|chunk| Ok(WireChunk(chunk)))
        .collect();
    if instance.failure {
        items.push_back(Err(StreamFailure));
    }
    ScriptedStream { items }
}

fn clock_for(instance: &IterateInstance) -> impl FnMut() -> DateTime<FixedOffset> {
    let end = instance.end_time;
    let mut times = VecDeque::from([instance.start_time, instance.end_time]);
    move || times.pop_front().unwrap_or(end)
}

async fn drive<Chunk, Source, Dispatch, Clock>(
    iterator: &mut A2AStreamingIterator<'_, Chunk, Source, Dispatch, Clock>,
) -> (u64, Option<&'static str>)
where
    Chunk: StreamedChunk + Clone,
    Source: Stream<Item = Result<Chunk, StreamFailure>> + Unpin,
    Dispatch: StreamSuccessDispatch,
    Clock: FnMut() -> DateTime<FixedOffset>,
{
    let mut yielded = 0_u64;
    let mut error = None;
    while let Some(item) = iterator.next_chunk::<StreamFailure>().await {
        if item.is_ok() {
            yielded = yielded.saturating_add(1);
        } else {
            error = Some("stream");
            break;
        }
    }
    (yielded, error)
}

async fn iterate(
    instance: &IterateInstance,
    counter: &Cl100kCounter,
) -> Result<AgentJson, Box<HelperError>> {
    let logging = state(&instance.logging)?;
    let mut iterator = A2AStreamingIterator::new(
        scripted(instance),
        instance.request.clone(),
        logging,
        StreamSetup {
            counter,
            max_retained_bytes: DEFAULT_MAX_RETAINED_BYTES,
            dispatch: Recorder::default(),
            clock: clock_for(instance),
        },
    );
    let (yielded, error) = drive(&mut iterator).await;
    let chunks: Vec<AgentJson> = iterator
        .chunks
        .iter()
        .map(|chunk| chunk.0.clone())
        .collect();
    let final_chunk = iterator.final_chunk.as_ref().map(|chunk| chunk.0.clone());
    let details = details_value(&iterator.logging)?;
    let dispatch = dispatch_value(&iterator.dispatch);
    Ok(serde_json::json!({
        "yielded": yielded,
        "chunks": chunks,
        "texts": iterator.collected_text_parts,
        "final_chunk": final_chunk,
        "details": details,
        "dispatch": dispatch,
        "error": error,
    }))
}

pub(super) fn run(instances: &[IterateInstance]) -> Result<AgentJson, Box<HelperError>> {
    let counter = Cl100kCounter::new()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| helper_error("RuntimeError", error.to_string()))?;
    let mut results = Vec::new();
    for instance in instances {
        results.push(runtime.block_on(iterate(instance, &counter))?);
    }
    Ok(serde_json::json!({ "results": results }))
}
