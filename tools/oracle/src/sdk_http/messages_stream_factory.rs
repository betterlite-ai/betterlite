use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::api::messages_stream_logging::StreamDeps;
use betterlite_core::api::messages_stream_logging::seams::{
    ChunkProcessorCall, LoggingFuture, ReportedFailure, RouteStreamingLogging, StreamSeams,
};
use betterlite_core::api::messages_stream_logging::slots::StreamSlots;
use betterlite_core::api::messages_thinking::stream::native_stream_factory;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::stream::Lease;
use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream::{Stream, try_unfold};
use indexmap::IndexMap;
use reqwest::Response;

use super::MessagesStreamFactoryCall;
use super::error::SdkHttpError;
use super::messages_header_stream::outbound;
use super::messages_stream_logging::outcome::Flag;
use super::result::{SdkResult, StreamFactoryRun};
use super::vector_query_embedding::outbound_client;

// drain awaits each chunk, so the boxed stream carries Send into the runner's future
type Chunks = Pin<Box<dyn Stream<Item = Result<Vec<u8>, SdkHttpError>> + Send>>;

// reqwest has no is_closed flag, so the flag records the drained stream releasing the live response.
struct Live {
    response: Response,
    released: Arc<AtomicBool>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.released.store(true, Ordering::SeqCst);
    }
}

// each chunk hands the response back so the next read polls it, and the last drop releases it
async fn next_chunk(mut live: Live) -> Result<Option<(Vec<u8>, Live)>, SdkHttpError> {
    let chunk = live
        .response
        .chunk()
        .await
        .map_err(|source| SdkHttpError::Stream(Box::new(HttpError::Request(source))))?;
    Ok(chunk.map(|bytes| (bytes.to_vec(), live)))
}

// B3609's hot path forwards each read as it arrives, so the double yields the live response's chunks.
struct FactorySeams {
    supplied: Arc<StreamLoggingObject>,
    processed: Mutex<Option<Processed>>,
}

struct Processed {
    request_body: IndexMap<String, Json>,
    endpoint_anthropic: bool,
    url_route: String,
    same_logging_object: bool,
    response_status: u16,
}

impl StreamSeams for FactorySeams {
    type Response = Live;
    type Stream = Chunks;

    fn route_streaming_logging(
        &self,
        _call: RouteStreamingLogging,
    ) -> Result<LoggingFuture, ReportedFailure> {
        Ok(Box::pin(async { Ok::<(), ReportedFailure>(()) }))
    }

    fn chunk_processor(&self, call: ChunkProcessorCall<Live>) -> Chunks {
        let processed = Processed {
            request_body: call.request_body,
            endpoint_anthropic: call.endpoint_type == EndpointType::Anthropic,
            url_route: call.url_route.to_owned(),
            same_logging_object: Arc::ptr_eq(&call.logging_obj, &self.supplied),
            response_status: call.response.response.status().as_u16(),
        };
        *self
            .processed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(processed);
        Box::pin(try_unfold(call.response, next_chunk))
    }
}

struct NoDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

struct FixedClock;

impl SuccessClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::UNIX_EPOCH.fixed_offset()
    }
}

struct Prepared {
    seams: Arc<FactorySeams>,
    supplied: Arc<StreamLoggingObject>,
    deps: StreamDeps<FactorySeams>,
    released: Arc<AtomicBool>,
    live: Live,
    body: IndexMap<String, Json>,
}

fn prepare(body: IndexMap<String, Json>, response: Response) -> Result<Prepared, SdkHttpError> {
    let supplied = Arc::new(StreamLoggingObject {
        slot: Mutex::new(None),
        dispatch: Arc::new(NoDispatch),
    });
    let seams = Arc::new(FactorySeams {
        supplied: Arc::clone(&supplied),
        processed: Mutex::new(None),
    });
    let deps = StreamDeps {
        seams: Arc::clone(&seams),
        worker: Arc::new(
            LoggingWorker::start(WorkerSettings::default()).map_err(SdkHttpError::Worker)?,
        ),
        slots: Arc::new(StreamSlots::default()),
        clock: Arc::new(FixedClock),
        success_handler: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
    };
    let released = Arc::new(AtomicBool::new(false));
    let live = Live {
        response,
        released: Arc::clone(&released),
    };
    Ok(Prepared {
        seams,
        supplied,
        deps,
        released,
        live,
        body,
    })
}

async fn drain(mut stream: Chunks) -> Result<Vec<u8>, SdkHttpError> {
    let mut payload = Vec::new();
    while let Some(chunk) = stream.next().await {
        payload.extend_from_slice(&chunk?);
    }
    Ok(payload)
}

fn assemble(
    seams: &FactorySeams,
    released: &AtomicBool,
    payload: Vec<u8>,
) -> Result<SdkResult, SdkHttpError> {
    let text = String::from_utf8(payload).map_err(SdkHttpError::FrameUtf8)?;
    let frames = text
        .split("\n\n")
        .filter(|frame| !frame.is_empty())
        .map(str::to_owned)
        .collect();
    let processed = seams
        .processed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .ok_or_else(|| {
            SdkHttpError::Invalid("the factory never reached the chunk processor".to_owned())
        })?;
    Ok(SdkResult::MessagesStreamFactory {
        value: StreamFactoryRun {
            frames,
            closed: Flag(released.load(Ordering::SeqCst)),
            request_body: processed.request_body,
            logging_object_passed: Flag(processed.same_logging_object),
            endpoint_anthropic: Flag(processed.endpoint_anthropic),
            url_route: processed.url_route,
            response_status: processed.response_status,
        },
    })
}

pub(crate) async fn send(
    call: &MessagesStreamFactoryCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let Json::Object(body) = &call.body else {
        return Err(SdkHttpError::Invalid(
            "the Messages factory request body must be an object".to_owned(),
        ));
    };
    let request = outbound(api_base, body)?;
    let client = outbound_client(env)?;
    let sent = client
        .send(request)
        .await
        .map_err(|error| SdkHttpError::Transport(Box::new(error)))?;
    let _lease: Lease = sent.lease;
    let prepared = prepare(body.clone(), sent.response)?;
    let stream = native_stream_factory(
        prepared.deps,
        prepared.live,
        prepared.body,
        prepared.supplied,
    );
    let payload = drain(stream).await?;
    assemble(&prepared.seams, &prepared.released, payload)
}
