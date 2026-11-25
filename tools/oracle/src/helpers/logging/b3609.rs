mod answer;
mod fakes;
mod input;
mod logging;

use std::mem::take;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_lifecycle::chunk_processor::{
    ChunkProcessorCall, chunk_processor,
};
use betterlite_logging::passthrough_stream_lifecycle::failure::StreamFailure;
use betterlite_logging::passthrough_stream_lifecycle::lifecycle::StreamLifecycle;
use betterlite_logging::passthrough_stream_lifecycle::ports::{
    DeferredStreamLogging, StreamLoggingJob,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::router::StreamLoggingRouter;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksHandler, EndpointType, ProviderChunkHandlers,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::DateTime;
use futures_util::stream;
use futures_util::{Stream, StreamExt as _};
use serde_json::Value;
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

pub(crate) struct Op;

impl Helper for Op {
    type Input = input::Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| invalid(&error.to_string()))?;
        runtime.block_on(execute(input))
    }
}

struct Wiring {
    lifecycle: Arc<StreamLifecycle>,
    recorder: Arc<fakes::Recorder>,
    logging_obj: Arc<StreamLoggingObject>,
}

async fn execute(input: &input::Input) -> HelperResult {
    let wiring = build_wiring(input)?;
    let mut source = Box::pin(chunk_processor(
        processor_call(input, &wiring)?,
        stream::iter(scripted_chunks(input)),
    ));
    let (yielded, failure, disconnected) = drive(input.disconnect_after, &mut source).await;
    drop(source);
    let queued_jobs = drain(&wiring.recorder.queued).await;
    let deferred_jobs = drain(&wiring.recorder.parked).await;
    let driven = answer::Driven {
        yielded,
        failure,
        disconnected,
        queued_jobs,
        deferred_jobs,
    };
    Ok(answer::answer(
        &wiring.logging_obj,
        &wiring.recorder,
        &driven,
    ))
}

fn build_wiring(input: &input::Input) -> Result<Wiring, Box<HelperError>> {
    let logging_obj = logging::logging_object()?;
    let recorder = Arc::new(fakes::Recorder::new(
        input.tinyfish_poller,
        input.run_id.clone(),
    ));
    let shared = Arc::new(fakes::PassThrough);
    let deferred_job: Arc<dyn DeferredStreamLogging> = Arc::<fakes::Recorder>::clone(&recorder);
    let deferred = input.deferred.then_some(deferred_job);
    let lifecycle = Arc::new(StreamLifecycle {
        tasks: TaskTracker::new(),
        router: router(&recorder, &shared),
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
        route: Arc::<fakes::Recorder>::clone(&recorder),
        cost_injection: Arc::<fakes::Recorder>::clone(&recorder),
        anthropic_partial_usage: Arc::<fakes::PassThrough>::clone(&shared),
        failure_dispatch: Arc::<fakes::Recorder>::clone(&recorder),
        queue: Arc::<fakes::Recorder>::clone(&recorder),
        binder: Arc::<fakes::Recorder>::clone(&recorder),
        deferred,
        clock: clock()?,
        cost_injection_enabled: input.cost_injection.0,
    });
    Ok(Wiring {
        lifecycle,
        recorder,
        logging_obj,
    })
}

fn router(
    recorder: &Arc<fakes::Recorder>,
    shared: &Arc<fakes::PassThrough>,
) -> Arc<StreamLoggingRouter> {
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(fakes::Handler);
    Arc::new(StreamLoggingRouter {
        providers: Arc::new(ProviderChunkHandlers {
            anthropic: Arc::clone(&handler),
            vertex: Arc::clone(&handler),
            gemini: Arc::clone(&handler),
            openai: handler,
        }),
        frames: Arc::<fakes::PassThrough>::clone(shared),
        tinyfish: Arc::<fakes::Recorder>::clone(recorder),
    })
}

fn clock() -> Result<Arc<fakes::FixedClock>, Box<HelperError>> {
    let moment = DateTime::parse_from_rfc3339("2024-01-02T00:00:00+00:00")
        .map_err(|error| invalid(&error.to_string()))?;
    Ok(Arc::new(fakes::FixedClock(moment)))
}

fn processor_call(
    input: &input::Input,
    wiring: &Wiring,
) -> Result<ChunkProcessorCall, Box<HelperError>> {
    Ok(ChunkProcessorCall {
        lifecycle: Arc::clone(&wiring.lifecycle),
        logging_obj: Arc::clone(&wiring.logging_obj),
        endpoint_type: EndpointType::from(input.endpoint_type),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .map_err(|error| invalid(&error.to_string()))?,
        url_route: input.url_route.clone(),
        request_body: request_body(input.request_body.as_ref())?,
        status_code: input.status_code,
    })
}

fn scripted_chunks(input: &input::Input) -> Vec<Result<Vec<u8>, StreamFailure>> {
    let mut items: Vec<Result<Vec<u8>, StreamFailure>> =
        input.chunks.iter().cloned().map(Ok).collect();
    if let Some(spec) = &input.failure {
        items.push(Err(StreamFailure::upstream(
            "ScriptedReadError".to_owned(),
            spec.message.clone(),
            spec.traceback.clone(),
            Arc::new(std::io::Error::other(spec.message.clone())),
        )));
    }
    items
}

async fn drain(jobs: &Mutex<Vec<StreamLoggingJob>>) -> usize {
    let queued = take(&mut *jobs.lock().unwrap_or_else(PoisonError::into_inner));
    let count = queued.len();
    for job in queued {
        job.work.await;
    }
    count
}

async fn drive<S>(
    disconnect_after: Option<usize>,
    chunks: &mut Pin<Box<S>>,
) -> (Vec<Vec<u8>>, Option<StreamFailure>, bool)
where
    S: Stream<Item = Result<Vec<u8>, StreamFailure>> + Send,
{
    let mut yielded = Vec::new();
    let mut failure = None;
    let mut disconnected = false;
    match disconnect_after {
        Some(limit) => {
            while yielded.len() < limit {
                match chunks.next().await {
                    Some(Ok(chunk)) => yielded.push(chunk),
                    Some(Err(error)) => {
                        failure = Some(error);
                        break;
                    }
                    None => break,
                }
            }
            disconnected = failure.is_none() && yielded.len() == limit;
        }
        None => {
            while let Some(item) = chunks.next().await {
                match item {
                    Ok(chunk) => yielded.push(chunk),
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
        }
    }
    (yielded, failure, disconnected)
}

fn request_body(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|body| {
            serde_json::from_value(body.clone()).map_err(|error| invalid(&error.to_string()))
        })
        .transpose()
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
