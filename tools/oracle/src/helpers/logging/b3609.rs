mod answer;
mod fakes;
mod input;
mod logging;

use std::mem::take;
use std::pin::Pin;
use std::sync::{Arc, PoisonError};

use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_lifecycle::chunk_processor::{
    ChunkProcessorCall, chunk_processor,
};
use betterlite_logging::passthrough_stream_lifecycle::cost_injection::StreamCostInjection;
use betterlite_logging::passthrough_stream_lifecycle::failure::{
    StreamFailure, StreamFailureDispatch,
};
use betterlite_logging::passthrough_stream_lifecycle::lifecycle::StreamLifecycle;
use betterlite_logging::passthrough_stream_lifecycle::partial_usage::AnthropicPartialUsage;
use betterlite_logging::passthrough_stream_lifecycle::ports::{
    DeferredStreamLogging, StreamLoggingQueue, StreamRoute,
};
use betterlite_logging::passthrough_stream_logging::router::{
    AnthropicFrameProbe, StreamLoggingRouter,
};
use betterlite_logging::passthrough_stream_logging::tinyfish::TinyfishStreamBilling;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksHandler, EndpointType, ProviderChunkHandlers,
};
use betterlite_logging::reservation_binding::ReservationBinder;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use chrono::DateTime;
use futures_util::stream;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use tokio::runtime::Builder;

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

async fn execute(input: &input::Input) -> HelperResult {
    let logging_obj = logging::logging_object()?;
    let handler: Arc<dyn CollectedChunksHandler> = Arc::new(fakes::Handler);
    let providers = Arc::new(ProviderChunkHandlers {
        anthropic: Arc::clone(&handler),
        vertex: Arc::clone(&handler),
        gemini: Arc::clone(&handler),
        openai: handler,
    });
    let recorder = Arc::new(fakes::Recorder::new(
        input.tinyfish_poller,
        input.run_id.clone(),
    ));
    let shared = Arc::new(fakes::PassThrough);
    let router = Arc::new(StreamLoggingRouter {
        providers,
        frames: Arc::clone(&shared) as Arc<dyn AnthropicFrameProbe>,
        tinyfish: Arc::clone(&recorder) as Arc<dyn TinyfishStreamBilling>,
    });
    let lifecycle = Arc::new(StreamLifecycle {
        router,
        passthrough: Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        }),
        route: Arc::clone(&recorder) as Arc<dyn StreamRoute>,
        cost_injection: Arc::clone(&recorder) as Arc<dyn StreamCostInjection>,
        anthropic_partial_usage: shared as Arc<dyn AnthropicPartialUsage>,
        failure_dispatch: Arc::clone(&recorder) as Arc<dyn StreamFailureDispatch>,
        queue: Arc::clone(&recorder) as Arc<dyn StreamLoggingQueue>,
        binder: Arc::clone(&recorder) as Arc<dyn ReservationBinder>,
        deferred: if input.deferred {
            Some(Arc::clone(&recorder) as Arc<dyn DeferredStreamLogging>)
        } else {
            None
        },
        clock: Arc::new(fakes::FixedClock(
            DateTime::parse_from_rfc3339("2024-01-02T00:00:00+00:00")
                .map_err(|error| invalid(&error.to_string()))?,
        )),
        cost_injection_enabled: input.cost_injection,
    });
    let call = ChunkProcessorCall {
        lifecycle: Arc::clone(&lifecycle),
        logging_obj: Arc::clone(&logging_obj),
        endpoint_type: EndpointType::from(input.endpoint_type),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .map_err(|error| invalid(&error.to_string()))?,
        url_route: input.url_route.clone(),
        request_body: request_body(input.request_body.as_ref())?,
        status_code: input.status_code,
    };
    let mut items: Vec<Result<Vec<u8>, StreamFailure>> =
        input.chunks.iter().cloned().map(Ok).collect();
    if let Some(spec) = &input.failure {
        items.push(Err(StreamFailure {
            class: "ScriptedReadError".to_owned(),
            message: spec.message.clone(),
            traceback: spec.traceback.clone(),
        }));
    }
    let mut chunks = Box::pin(chunk_processor(call, stream::iter(items)));
    let (yielded, failure, disconnected) = drive(input.disconnect_after, &mut chunks).await;
    drop(chunks);
    let queued = take(
        &mut *recorder
            .queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let queued_jobs = queued.len();
    for job in queued {
        job.work.await;
    }
    let parked = take(
        &mut *recorder
            .parked
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let deferred_jobs = parked.len();
    for job in parked {
        job.work.await;
    }
    Ok(answer::answer(
        &logging_obj,
        &recorder,
        answer::Driven {
            yielded,
            failure,
            disconnected,
            queued_jobs,
            deferred_jobs,
        },
    ))
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
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|error| invalid(&error.to_string()))
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
