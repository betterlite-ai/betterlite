mod hooks;
mod late;

use std::io;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use axum::response::Response;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_exception::ProxyException;
use betterlite_http::error::HttpError;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::routing_timeouts::plan::RouterTimeouts;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use self::hooks::QueueHooks;
use self::late::{QueueLateFailure, exception_from_sse};
use super::super::state::{QueueCompletion, QueueFailureHook, QueueRouter, timeout_plan};
use super::request::QueueRequest;
use super::scheduled::{QueueError, QueuedCompletion, report, scheduler_item};
use crate::data_generator_select::SelectDataGenerator;
use crate::first_byte_sse::{
    LateFailureHook, ProducedResponse, StreamedResponse, open_sse_before_first_byte,
};
use crate::keepalive_interval::{PingSetting, ttft_keepalive_interval};
use crate::messages_first_chunk::answer::sse_stream_headers;
use crate::sse_data_generator::types::{
    CleanupOutcome, ErrorStateSource, SseDataGenerator, StreamCleanup, StreamErrorState,
    StreamFailure, StreamSource,
};
use crate::sse_error_frames::SseFailure;
use crate::stream_chunks::StreamItem;

mod tests;

const MEDIA_TYPE: &str = "text/event-stream";

pub(super) async fn streamed(
    request: QueueRequest<'_>,
    router: &QueueRouter,
    data: &JsonMap,
) -> Result<Response, QueueError> {
    let input = streamed_queue(request, router, data);
    let keepalive_request = JsonValue::Object(input.data.clone());
    let interval = ttft_keepalive_interval(&keepalive_request, &[], input.keepalive.as_ref())
        .map(|duration| PingSetting::Seconds(duration.as_secs_f64()));
    let late = input
        .failure
        .as_ref()
        .map(|hook| -> Arc<dyn LateFailureHook> {
            Arc::new(QueueLateFailure {
                hook: Arc::clone(hook),
                caller: input.caller.clone(),
                data: input.data.clone(),
            })
        });
    let failure = input.failure.clone();
    let caller = input.caller.clone();
    let request_data = input.data.clone();
    let produced = async move {
        let source = open_source(&input).await?;
        Ok(streaming_answer(&input, source))
    };
    match open_sse_before_first_byte(produced, interval, MEDIA_TYPE, late).await {
        Ok(response) => Ok(response),
        Err(error) => Ok(report(
            failure.as_deref(),
            &caller,
            &request_data,
            exception_from_sse(&error),
        )
        .await),
    }
}

struct StreamedQueue {
    scheduler: Arc<RequestScheduler>,
    completion: Arc<dyn QueueCompletion>,
    failure: Option<Arc<dyn QueueFailureHook>>,
    timeouts: RouterTimeouts,
    keepalive: Option<PingSetting>,
    caller: Principal,
    data: JsonMap,
}

fn streamed_queue(
    request: QueueRequest<'_>,
    router: &QueueRouter,
    data: &JsonMap,
) -> StreamedQueue {
    StreamedQueue {
        scheduler: Arc::clone(&router.scheduler),
        completion: Arc::clone(&router.completion),
        failure: request.queue.failure.as_ref().map(Arc::clone),
        timeouts: request.queue.timeouts,
        keepalive: request.queue.keepalive.clone(),
        caller: request.caller.clone(),
        data: data.clone(),
    }
}

// Router.schedule_acompletion for a streaming call: the call's own answer is the iterator the generator drives
async fn open_source(input: &StreamedQueue) -> Result<StreamSource, SseFailure> {
    let adapter = QueuedCompletion {
        completion: input.completion.as_ref(),
        caller: &input.caller,
        data: &input.data,
        stream: true,
        source: Mutex::new(None),
    };
    let item = scheduler_item(input.scheduler.as_ref(), &input.data)
        .map_err(|error| sse_failure(&error.exception()))?;
    let plan = timeout_plan(&input.timeouts, true)
        .map_err(|error| sse_failure(&QueueError::Timeout(error).exception()))?;
    if let Err(error) = input.scheduler.schedule(&item, &adapter, &plan).await {
        return Err(sse_failure(&QueueError::Scheduler(error).exception()));
    }
    adapter
        .take_source()
        .ok_or_else(|| sse_failure(&QueueError::EmptyAnswer.exception()))
}

fn streaming_answer(input: &StreamedQueue, source: StreamSource) -> ProducedResponse {
    let generator: SseDataGenerator<Principal> = SseDataGenerator {
        hooks: Arc::new(QueueHooks {
            failure: input.failure.as_ref().map(Arc::clone),
        }),
        error_state: Arc::new(NoQueueStream),
        cleanup: Arc::new(NoQueueStream),
        router: None,
        global_keepalive: input.keepalive.clone(),
        expose_fallback_errors: false,
        is_model_router: |_model| false,
    };
    let selector = SelectDataGenerator {
        generator,
        responses_stream_errors: false,
    };
    let items = selector.select(
        source,
        input.caller.clone(),
        input.data.clone().into_iter().collect(),
        None,
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    ProducedResponse::Streaming(StreamedResponse {
        status: StatusCode::OK,
        headers: sse_stream_headers(headers),
        body: stream::unfold(items, next_frame).boxed(),
    })
}

type GeneratorStream = BoxStream<'static, Result<Bytes, io::Error>>;

async fn next_frame(
    mut items: GeneratorStream,
) -> Option<(Result<Bytes, HttpError>, GeneratorStream)> {
    match items.next().await {
        Some(Ok(bytes)) => Some((Ok(bytes), items)),
        Some(Err(error)) => {
            tracing::error!("the queued stream ended after an error: {error}");
            None
        }
        None => None,
    }
}

// the queued route renders its own error frames, so the generator's error state and cleanup stay idle
struct NoQueueStream;

impl ErrorStateSource for NoQueueStream {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        Box::new(Self)
    }
}

impl StreamErrorState for NoQueueStream {
    fn observe_chunk(&mut self, _item: &StreamItem) {}

    fn mark_emitted(&mut self, frame: String) -> String {
        frame
    }

    fn format_failure(&mut self, _failure: &StreamFailure) -> Option<String> {
        None
    }
}

impl StreamCleanup<Principal> for NoQueueStream {
    fn finalize(&self, _outcome: CleanupOutcome<Principal>) {}
}

fn sse_failure(exception: &ProxyException) -> SseFailure {
    SseFailure {
        status_code: exception.code.parse::<u16>().ok(),
        code: Some(exception.code.clone()),
        error_type: Some(exception.error_type.clone()),
        param: exception.param.clone(),
        detail: Some(Box::new(JsonValue::String(exception.message.clone()))),
        provider_specific_fields: None,
    }
}
