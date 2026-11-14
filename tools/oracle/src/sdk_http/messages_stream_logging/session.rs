use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_core::api::messages_stream_logging::seams::{LoggingFuture, LoggingJob};
use betterlite_core::api::messages_stream_logging::slots::{Fields, StreamSlots};
use betterlite_core::api::messages_stream_logging::{
    MessagesStreamLogging, SseChunk, StreamDeps, abort_upstream, convert_chunk_to_sse_format,
};
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use betterlite_models::decisions::Json;
use chrono::DateTime;
use indexmap::IndexMap;

use super::Http;
use super::call::{ChunkSpec, Hook, LoggingStep};
use super::doubles::{RecordingHook, Upstream};
use super::outcome::{Processed, Routed, StepOutcome};
use super::recording::{
    Recorder, StepClock, Warnings, counted_future, logging_object, value_error,
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
        let recorder = Arc::new(Recorder {
            routed: Mutex::new(Vec::new()),
            processed: Mutex::new(Vec::new()),
            route_failure: Mutex::new(None),
            ran: Arc::new(AtomicUsize::new(0)),
        });
        let worker = LoggingWorker::start(WorkerSettings::default())
            .map_err(|error| SdkHttpError::Logging(error.to_string()))?;
        let worker = Arc::new(worker);
        let slots = Arc::new(StreamSlots::default());
        let handler = Arc::new(PassThroughEndpointLogging {
            injected_log_dispatch: None,
        });
        let logging_obj = logging_object()?;
        let stream = MessagesStreamLogging::new(
            StreamDeps {
                seams: Arc::clone(&recorder),
                worker: Arc::clone(&worker),
                slots: Arc::clone(&slots),
                clock: Arc::new(StepClock::new()),
                success_handler: Arc::clone(&handler),
            },
            Arc::clone(&logging_obj),
            body.clone(),
        );
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
        match step {
            LoggingStep::Collect { first_chunk } => {
                self.frames = http.frames(&self.body).await?;
                if *first_chunk {
                    self.stream.record_first_chunk();
                }
            }
            LoggingStep::Log { teardown } => {
                let frames = self.frames.clone();
                self.stream
                    .handle_streaming_logging(frames, *teardown)
                    .map_err(|failure| SdkHttpError::Logging(failure.to_string()))?;
            }
            LoggingStep::Bill {
                teardown,
                route_failure,
            } => {
                *self
                    .recorder
                    .route_failure
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) =
                    route_failure.as_deref().map(value_error);
                self.stream
                    .bill_collected_chunks(self.frames.clone(), *teardown);
            }
            LoggingStep::ArmDeferred => self.arm_deferred(),
            LoggingStep::ParkArgs { count } => {
                let parked: Vec<LoggingFuture> = (0..*count)
                    .map(|_| counted_future(&self.recorder.ran))
                    .collect();
                self.fields().deferred_args = Some(parked);
            }
            LoggingStep::Dispatch => {
                self.stream.dispatch_pending_logging();
            }
            LoggingStep::AbortUpstream { close } => {
                abort_upstream(&mut Upstream { close: *close }).await;
            }
            LoggingStep::DetachedFailure { hook } => {
                let calls = Arc::clone(&self.hook_calls);
                self.fields().detached_failure_hook = match hook {
                    Hook::Missing => None,
                    Hook::Records => Some(Arc::new(RecordingHook {
                        calls,
                        raises: false,
                    })),
                    Hook::Raises => Some(Arc::new(RecordingHook {
                        calls,
                        raises: true,
                    })),
                };
                let original = value_error("original");
                self.stream.fire_detached_failure_hook(&original).await;
            }
            LoggingStep::Convert { chunk } => {
                let chunk = match chunk {
                    ChunkSpec::Mapping(mapping) => SseChunk::Mapping(mapping.clone()),
                    ChunkSpec::Bytes(bytes) => SseChunk::Bytes(bytes.clone().into_bytes()),
                };
                let frame = convert_chunk_to_sse_format(chunk)
                    .map_err(|error| SdkHttpError::Logging(error.to_string()))?;
                outcome.frame = Some(String::from_utf8_lossy(&frame).into_owned());
            }
            LoggingStep::Delegate => self.delegate(http, &mut outcome).await?,
        }
        self.worker.flush().await;
        self.observe(&mut outcome);
        Ok(outcome)
    }

    fn fields(&self) -> MutexGuard<'_, Fields> {
        self.slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn arm_deferred(&self) {
        let args = Arc::clone(&self.callback_args);
        let ran = Arc::clone(&self.recorder.ran);
        self.fields().deferred_callback = Some(Box::new(move |parked: Vec<LoggingFuture>| {
            args.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(parked.len());
            counted_future(&ran)
        }));
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
        outcome.provider_text = Some(String::from_utf8_lossy(&relayed_bytes).into_owned());
        let calls = std::mem::take(
            &mut *self
                .recorder
                .processed
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        outcome.processed = calls
            .into_iter()
            .map(|call| Processed {
                response_status: call.response,
                request_body: call.request_body,
                endpoint_anthropic: call.endpoint_type == EndpointType::Anthropic,
                url_route: call.url_route.to_owned(),
                supplied_logging_obj: Arc::ptr_eq(&call.logging_obj, &supplied),
                shared_success_handler: Arc::ptr_eq(&call.success_handler, &self.handler),
                start_is_construction: call.start_time == DateTime::UNIX_EPOCH.fixed_offset(),
            })
            .collect();
        Ok(())
    }

    fn observe(&self, outcome: &mut StepOutcome) {
        outcome.collected = self.frames.len();
        outcome.ran = self.recorder.ran.swap(0, Ordering::SeqCst);
        outcome.warnings = self.warnings.take();
        outcome.callback_args = std::mem::take(
            &mut *self
                .callback_args
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        outcome.hook_calls = std::mem::take(
            &mut *self
                .hook_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
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
        outcome.completion_start_recorded = recorded;
        let fields = self.fields();
        outcome.callback_armed = fields.deferred_callback.is_some();
        outcome.args_present = fields.deferred_args.is_some();
        drop(fields);
        let calls = std::mem::take(
            &mut *self
                .recorder
                .routed
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        );
        outcome.routed = calls
            .into_iter()
            .map(|call| Routed {
                raw_frames: call
                    .raw_bytes
                    .iter()
                    .map(|frame| String::from_utf8_lossy(frame).into_owned())
                    .collect(),
                request_body: call.request_body,
                endpoint_anthropic: call.endpoint_type == EndpointType::Anthropic,
                url_route: call.url_route.to_owned(),
                iterator_logging_obj: Arc::ptr_eq(&call.logging_obj, &self.logging_obj),
                shared_success_handler: Arc::ptr_eq(&call.success_handler, &self.handler),
                start_is_construction: call.start_time == DateTime::UNIX_EPOCH.fixed_offset(),
                end_not_before_start: call.end_time >= call.start_time,
                first_within_run: first
                    .map(|first| call.start_time <= first && first <= call.end_time),
            })
            .collect();
    }
}
