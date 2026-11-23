use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use axum::response::Response;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::settings::JsonValue;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_http::error::HttpError;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::request_scheduler::service::RequestScheduler;
use betterlite_router::routing_timeouts::plan::RouterTimeouts;
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::{self, BoxStream};

use super::super::state::{QueueCompletion, QueueFailureHook, QueueRouter, timeout_plan};
use super::request::{QueueRequest, data_map};
use super::scheduled::{QueueError, QueuedCompletion, report, scheduler_item};
use crate::data_generator_select::SelectDataGenerator;
use crate::first_byte_sse::{
    ByteStream, HookRaised, LateFailureHook, ProducedResponse, StreamedResponse,
    open_sse_before_first_byte,
};
use crate::keepalive_interval::{PingSetting, ttft_keepalive_interval};
use crate::messages_first_chunk::answer::sse_stream_headers;
use crate::sse_data_generator::types::{
    CleanupOutcome, ErrorStateSource, FailureKind, FailureParam, FailureType, ItemStream,
    SseDataGenerator, StreamCleanup, StreamErrorState, StreamFailure, StreamHooks, StreamSource,
};
use crate::sse_error_frames::{SseFailure, exception_detail_parts};
use crate::stream_chunks::StreamItem;

const MEDIA_TYPE: &str = "text/event-stream";

pub(super) async fn streamed(
    request: QueueRequest<'_>,
    router: &QueueRouter,
    data: &JsonMap,
) -> Result<Response, QueueError> {
    let input = StreamedQueue {
        scheduler: Arc::clone(&router.scheduler),
        completion: Arc::clone(&router.completion),
        failure: request.queue.failure.as_ref().map(Arc::clone),
        timeouts: request.queue.timeouts,
        keepalive: request.queue.keepalive.clone(),
        caller: request.caller.clone(),
        data: data.clone(),
    };
    let interval = keepalive_interval(input.keepalive.as_ref(), &input.data);
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
    match open_sse_before_first_byte(produce(input), interval, MEDIA_TYPE, late).await {
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

async fn produce(input: StreamedQueue) -> Result<ProducedResponse, SseFailure> {
    let source = open_source(&input).await?;
    Ok(streaming_answer(&input, source))
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
        error_state: Arc::new(NoQueueErrorState),
        cleanup: Arc::new(NoQueueCleanup),
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
        generator_request_data(&input.data),
        None,
    );
    ProducedResponse::Streaming(StreamedResponse {
        status: StatusCode::OK,
        headers: stream_headers(),
        body: relay(items),
    })
}

fn generator_request_data(data: &JsonMap) -> BTreeMap<String, JsonValue> {
    data.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn stream_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    sse_stream_headers(headers)
}

type GeneratorStream = BoxStream<'static, Result<Bytes, io::Error>>;

// the generator reports a failure the relay cannot carry as an io error, so the stream ends there
fn relay(items: GeneratorStream) -> ByteStream {
    stream::unfold(items, next_frame).boxed()
}

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

struct QueueHooks {
    failure: Option<Arc<dyn QueueFailureHook>>,
}

#[async_trait]
impl StreamHooks<Principal> for QueueHooks {
    fn needs_iterator_wrap(&self) -> bool {
        false
    }

    fn needs_per_chunk_hook(&self) -> bool {
        false
    }

    fn wrap_iterator(
        &self,
        items: ItemStream,
        _key: &Principal,
        _request_data: &JsonObject,
    ) -> ItemStream {
        items
    }

    async fn chunk_hook(
        &self,
        item: StreamItem,
        _key: &Principal,
        _request_data: &JsonObject,
        _str_so_far: Option<&str>,
    ) -> Result<StreamItem, StreamFailure> {
        Ok(item)
    }

    async fn failure_hook(
        &self,
        key: &Principal,
        failure: &StreamFailure,
        request_data: &JsonObject,
    ) -> Result<(), StreamFailure> {
        let Some(hook) = &self.failure else {
            return Ok(());
        };
        let exception = exception_from_stream(failure);
        match hook
            .post_call_failure(key, &exception, &data_map(request_data))
            .await
        {
            Some(replacement) => Err(stream_failure(&replacement)),
            None => Ok(()),
        }
    }

    fn fire_deferred_stream_logging(&self, _request_data: &JsonObject) {}
}

struct QueueLateFailure {
    hook: Arc<dyn QueueFailureHook>,
    caller: Principal,
    data: JsonMap,
}

#[async_trait]
impl LateFailureHook for QueueLateFailure {
    async fn late_failure(&self, failure: &SseFailure) -> Result<Option<SseFailure>, HookRaised> {
        let exception = exception_from_sse(failure);
        let answered = self
            .hook
            .post_call_failure(&self.caller, &exception, &self.data)
            .await
            .as_ref()
            .map(sse_failure);
        Ok(answered)
    }
}

struct NoQueueErrorState;

impl ErrorStateSource for NoQueueErrorState {
    fn new_state(&self) -> Box<dyn StreamErrorState + Send> {
        Box::new(NoQueueStreamState)
    }
}

struct NoQueueStreamState;

impl StreamErrorState for NoQueueStreamState {
    fn observe_chunk(&mut self, _item: &StreamItem) {}

    fn mark_emitted(&mut self, frame: String) -> String {
        frame
    }

    fn format_failure(&mut self, _failure: &StreamFailure) -> Option<String> {
        None
    }
}

struct NoQueueCleanup;

impl StreamCleanup<Principal> for NoQueueCleanup {
    fn finalize(&self, _outcome: CleanupOutcome<Principal>) {}
}

fn keepalive_interval(global: Option<&PingSetting>, data: &JsonMap) -> Option<PingSetting> {
    let request = JsonValue::Object(data.clone());
    ttft_keepalive_interval(&request, &[], global)
        .map(|duration| PingSetting::Seconds(duration.as_secs_f64()))
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

fn exception_from_sse(failure: &SseFailure) -> ProxyException {
    let message = failure
        .detail
        .as_deref()
        .map_or_else(String::new, |detail| exception_detail_parts(detail).0);
    let code = failure.code.clone().map_or_else(
        || {
            failure.status_code.map_or(ExceptionCode::Absent, |status| {
                ExceptionCode::Status(i64::from(status))
            })
        },
        ExceptionCode::Text,
    );
    ProxyException::new(ProxyExceptionInit {
        message,
        error_type: failure
            .error_type
            .clone()
            .unwrap_or_else(|| ProxyErrorTypes::AuthError.to_string()),
        param: failure.param.clone(),
        code,
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn exception_from_stream(failure: &StreamFailure) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: failure.message.clone(),
        error_type: match &failure.error_type {
            FailureType::Text(text) => text.clone(),
            FailureType::Missing | FailureType::Null => ProxyErrorTypes::AuthError.to_string(),
        },
        param: match &failure.param {
            FailureParam::Text(text) => Some(text.clone()),
            FailureParam::Missing | FailureParam::Null => None,
        },
        code: failure.status_code.map_or(ExceptionCode::Absent, |status| {
            ExceptionCode::Status(i64::from(status))
        }),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

fn stream_failure(exception: &ProxyException) -> StreamFailure {
    StreamFailure {
        message: exception.message.clone(),
        error_type: FailureType::Text(exception.error_type.clone()),
        param: match &exception.param {
            Some(text) => FailureParam::Text(text.clone()),
            None => FailureParam::Null,
        },
        status_code: exception.code.parse::<u16>().ok(),
        code: None,
        body: None,
        kind: FailureKind::Other,
    }
}
