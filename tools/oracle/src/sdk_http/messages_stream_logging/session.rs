use std::iter::repeat_with;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_core::api::messages_stream_logging::seams::{
    LoggingFuture, LoggingJob, ReportedFailure, RouteStreamingLogging,
};
use betterlite_core::api::messages_stream_logging::slots::{Fields, StreamSlots};
use betterlite_core::api::messages_stream_logging::{
    MessagesStreamLogging, SseChunk, StreamDeps, abort_upstream, convert_chunk_to_sse_format,
};
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;

use super::Http;
use super::call::{ChunkSpec, Hook, LoggingStep};
use super::doubles::{RecordingHook, Upstream};
use super::outcome::{Flag, Processed, Routed, StepOutcome};
use super::recording::{
    Recorder, StepClock, Warnings, counted_future, drain, logging_object, value_error,
};
use crate::sdk_http::error::SdkHttpError;

pub(super) struct Session {
    recorder: Arc<Recorder>,
    worker: Arc<LoggingWorker<LoggingJob>>,
    slots: Arc<StreamSlots>,
    handler: Arc<PassThroughEndpointLogging>,
    logging_obj: Arc<StreamLoggingObject>,
    stream: MessagesStreamLogging<Recorder>,
    body: IndexMap<String, Json>,
    frames: Vec<Vec<u8>>,
    hook_calls: Arc<Mutex<Vec<String>>>,
    callback_args: Arc<Mutex<Vec<usize>>>,
    warnings: Warnings,
}

impl Session {
    pub(super) fn start(
        body: IndexMap<String, Json>,
        warnings: Warnings,
    ) -> Result<Self, SdkHttpError> {
        let recorder = Arc::new(Recorder::default());
        let logging_worker =
            LoggingWorker::start(WorkerSettings::default()).map_err(SdkHttpError::Worker)?;
        let worker = Arc::new(logging_worker);
        let slots = Arc::new(StreamSlots::default());
        let handler = Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        });
        let logging_obj = logging_object()?;
        let deps = StreamDeps {
            seams: Arc::clone(&recorder),
            worker: Arc::clone(&worker),
            slots: Arc::clone(&slots),
            clock: Arc::new(StepClock::default()),
            success_handler: Arc::clone(&handler),
        };
        let stream = MessagesStreamLogging::new(deps, Arc::clone(&logging_obj), body.clone());
        Ok(Self {
            recorder,
            worker,
            slots,
            handler,
            logging_obj,
            stream,
            body,
            frames: Vec::new(),
            hook_calls: Arc::new(Mutex::new(Vec::new())),
            callback_args: Arc::new(Mutex::new(Vec::new())),
            warnings,
        })
    }

    pub(super) async fn run(
        &mut self,
        step: &LoggingStep,
        http: &Http<'_>,
    ) -> Result<StepOutcome, SdkHttpError> {
        let mut outcome = StepOutcome::default();
        self.apply(step, http, &mut outcome).await?;
        self.worker.flush().await;
        self.observe(&mut outcome)?;
        Ok(outcome)
    }

    async fn apply(
        &mut self,
        step: &LoggingStep,
        http: &Http<'_>,
        outcome: &mut StepOutcome,
    ) -> Result<(), SdkHttpError> {
        match step {
            LoggingStep::Collect { first_chunk } => self.collect(http, *first_chunk).await?,
            LoggingStep::Bill {
                teardown,
                route_failure,
            } => self.bill(*teardown, route_failure.as_deref()),
            LoggingStep::ArmDeferred => self.arm_deferred(),
            LoggingStep::ParkArgs { count } => self.park_args(*count),
            LoggingStep::Dispatch => {
                let _dispatched = self.stream.dispatch_pending_logging();
            }
            LoggingStep::AbortUpstream { close } => {
                abort_upstream(&mut Upstream { close: *close }).await;
            }
            LoggingStep::DetachedFailure { hook } => self.detached_failure(*hook).await,
            LoggingStep::Convert { chunk } => outcome.text = Some(convert(chunk)?),
            LoggingStep::Delegate => self.delegate(http, outcome).await?,
        }
        Ok(())
    }

    async fn collect(&mut self, http: &Http<'_>, first_chunk: bool) -> Result<(), SdkHttpError> {
        self.frames = http.frames(&self.body).await?;
        if first_chunk {
            self.stream.record_first_chunk();
        }
        Ok(())
    }

    fn bill(&self, teardown: bool, failure: Option<&str>) {
        *self
            .recorder
            .route_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = failure.map(value_error);
        let _logged = self
            .stream
            .bill_collected_chunks(self.frames.clone(), teardown);
    }

    async fn detached_failure(&self, hook: Hook) {
        let recording = RecordingHook {
            calls: Arc::clone(&self.hook_calls),
            raises: hook == Hook::Raises,
        };
        self.fields().detached_failure_hook = if hook == Hook::Missing {
            None
        } else {
            Some(Arc::new(recording))
        };
        let original = value_error("original");
        self.stream.fire_detached_failure_hook(&original).await;
    }

    fn fields(&self) -> MutexGuard<'_, Fields> {
        self.slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn arm_deferred(&self) {
        let args = Arc::clone(&self.callback_args);
        self.fields().deferred_callback = Some(Box::new(move |parked: Vec<LoggingFuture>| {
            args.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(parked.len());
            Box::pin(await_parked(parked))
        }));
    }

    fn park_args(&self, count: usize) {
        let parked: Vec<LoggingFuture> = repeat_with(|| counted_future(&self.recorder.ran))
            .take(count)
            .collect();
        self.fields().deferred_args = Some(parked);
    }

    // The processor gets a logging object and body of its own, so the caller's own are told apart
    async fn delegate(
        &self,
        http: &Http<'_>,
        outcome: &mut StepOutcome,
    ) -> Result<(), SdkHttpError> {
        let supplied = logging_object()?;
        let supplied_body =
            IndexMap::from([("model".to_owned(), Json::Text("supplied".to_owned()))]);
        let (response, _lease) = http.post(&self.body).await?;
        let relayed =
            self.stream
                .streaming_response_iterator(response, supplied_body, Arc::clone(&supplied));
        let relayed_bytes = Http::read(relayed).await?;
        outcome.text = Some(String::from_utf8(relayed_bytes).map_err(SdkHttpError::FrameUtf8)?);
        outcome.processed = drain(&self.recorder.processed)
            .into_iter()
            .map(|call| Processed {
                response_status: call.response,
                request_body: call.request_body,
                endpoint_anthropic: Flag(call.endpoint_type == EndpointType::Anthropic),
                url_route: call.url_route.to_owned(),
                supplied_logging_obj: Flag(Arc::ptr_eq(&call.logging_obj, &supplied)),
                shared_success_handler: Flag(Arc::ptr_eq(&call.success_handler, &self.handler)),
                start_is_construction: Flag(call.start_time == DateTime::UNIX_EPOCH.fixed_offset()),
            })
            .collect();
        Ok(())
    }

    fn routed_entry(
        &self,
        call: RouteStreamingLogging,
        first: Option<DateTime<FixedOffset>>,
    ) -> Result<Routed, SdkHttpError> {
        let raw_frames = call
            .raw_bytes
            .iter()
            .map(|frame| String::from_utf8(frame.clone()).map_err(SdkHttpError::FrameUtf8))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Routed {
            raw_frames,
            request_body: call.request_body,
            endpoint_anthropic: Flag(call.endpoint_type == EndpointType::Anthropic),
            url_route: call.url_route.to_owned(),
            iterator_logging_obj: Flag(Arc::ptr_eq(&call.logging_obj, &self.logging_obj)),
            shared_success_handler: Flag(Arc::ptr_eq(&call.success_handler, &self.handler)),
            start_is_construction: Flag(call.start_time == DateTime::UNIX_EPOCH.fixed_offset()),
            first_within_run: first.map(|start| call.start_time <= start && start <= call.end_time),
        })
    }

    fn observe(&self, outcome: &mut StepOutcome) -> Result<(), SdkHttpError> {
        outcome.ran = self.recorder.ran.swap(0, Ordering::SeqCst);
        outcome.warnings = self.warnings.take()?;
        outcome.callback_args = drain(&self.callback_args);
        outcome.hook_calls = drain(&self.hook_calls);
        let (first, recorded) = self
            .logging_obj
            .read(&mut |state| {
                (
                    state.completion_start_time,
                    state.completion_start_time.is_some()
                        && state
                            .model_call_details
                            .contains_key("completion_start_time"),
                )
            })
            .unwrap_or_default();
        outcome.completion_start_recorded = Flag(recorded);
        let fields = self.fields();
        outcome.callback_armed = Flag(fields.deferred_callback.is_some());
        outcome.args_present = Flag(fields.deferred_args.is_some());
        drop(fields);
        outcome.routed = drain(&self.recorder.routed)
            .into_iter()
            .map(|call| self.routed_entry(call, first))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }
}

// The deferred callback awaits the parked futures in the order LiteLLM hands them over
async fn await_parked(parked: Vec<LoggingFuture>) -> Result<(), ReportedFailure> {
    for future in parked {
        future.await?;
    }
    Ok(())
}

fn convert(chunk: &ChunkSpec) -> Result<String, SdkHttpError> {
    let sse = match chunk {
        ChunkSpec::Mapping(mapping) => SseChunk::Mapping(mapping.clone()),
        ChunkSpec::Bytes(bytes) => SseChunk::Bytes(bytes.as_bytes().to_vec()),
    };
    let frame = convert_chunk_to_sse_format(sse).map_err(SdkHttpError::Chunk)?;
    String::from_utf8(frame).map_err(SdkHttpError::FrameUtf8)
}
