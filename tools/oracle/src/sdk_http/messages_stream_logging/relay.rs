use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::messages_native_stream::registry::StreamRegistry;
use betterlite_core::api::messages_native_stream::{
    ClientRelay, RelayLimits, RelaySetup, start_relay,
};
use betterlite_core::api::messages_stream_logging::StreamDeps;
use betterlite_core::api::messages_stream_logging::seams::{
    LoggingFuture, LoggingJob, ReportedFailure,
};
use betterlite_core::api::messages_stream_logging::slots::StreamSlots;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::types::EndpointType;
use betterlite_logging::worker::{LoggingWorker, WorkerSettings};
use indexmap::IndexMap;
use tracing::subscriber::set_default;

use super::Http;
use super::outcome::Flag;
use super::recording::{StepClock, Warnings, drain, logging_object};
use super::session::await_parked;
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::SdkResult;
use crate::sdk_http::vector_query_embedding::outbound_client;
use call::{
    FailureCall, FrameRead, MessagesNativeStreamCall, RelayOp, RelayStep, StreamObservation,
    WaitUntil,
};
use run::Run;
use state::{Gate, GateSource, StreamState, quiesce};

pub mod call;
mod run;
mod state;

pub(crate) async fn send(
    call: &MessagesNativeStreamCall,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let warnings = Warnings::default();
    let writer = warnings.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_target(false)
        .with_level(false)
        .finish();
    let _guard = set_default(subscriber);
    let http = Http {
        client: outbound_client(env)?,
        api_base,
    };
    let mut run = Run {
        call,
        http: &http,
        registry: Arc::new(StreamRegistry::default()),
        warnings,
        sessions: IndexMap::new(),
    };
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(run.apply(step).await?);
    }
    Ok(SdkResult::MessagesNativeStream { value: steps })
}

struct Session {
    state: Arc<StreamState>,
    worker: Arc<LoggingWorker<LoggingJob>>,
    slots: Arc<StreamSlots>,
    gate: Arc<Gate>,
    relay: Option<ClientRelay<StreamState, ReportedFailure>>,
    callback_args: Arc<Mutex<Vec<usize>>>,
}

impl Session {
    async fn start(
        call: &MessagesNativeStreamCall,
        http: &Http<'_>,
        registry: Arc<StreamRegistry>,
    ) -> Result<Self, SdkHttpError> {
        let state = Arc::new(StreamState::default());
        let worker = Arc::new(
            LoggingWorker::start(WorkerSettings::default()).map_err(SdkHttpError::Worker)?,
        );
        let slots = Arc::new(StreamSlots::default());
        let gate = Arc::new(Gate::default());
        let source = gate_source(http, call, &gate).await?;
        install_failure_hook(&slots, &state);
        let setup = relay_setup(call, &state, &worker, &slots, registry)?;
        Ok(Self {
            state,
            worker,
            slots,
            gate,
            relay: Some(start_relay(source, setup).relay),
            callback_args: Arc::new(Mutex::new(Vec::new())),
        })
    }

    async fn apply(&mut self, step: &RelayStep) -> Result<Option<FrameRead>, SdkHttpError> {
        match step.op {
            RelayOp::Read => Ok(Some(self.read().await?)),
            RelayOp::Release => {
                self.gate
                    .state
                    .send_modify(|state| state.permits = state.permits.saturating_add(step.count));
                Ok(None)
            }
            RelayOp::Detach => {
                self.relay = None;
                Ok(None)
            }
            RelayOp::Finish => {
                self.gate.state.send_modify(|state| state.finished = true);
                Ok(None)
            }
            RelayOp::Fail => {
                self.gate.state.send_modify(|state| {
                    state.failure = Some(ReportedFailure {
                        kind: "RateLimitError".to_owned(),
                        message: "busy".to_owned(),
                    });
                });
                Ok(None)
            }
            RelayOp::Arm => {
                self.arm();
                Ok(None)
            }
            RelayOp::Wait => {
                self.wait(step.until).await;
                Ok(None)
            }
            RelayOp::Start | RelayOp::Settle => Ok(None),
        }
    }

    async fn read(&mut self) -> Result<FrameRead, SdkHttpError> {
        let Some(relay) = self.relay.as_mut() else {
            return Err(SdkHttpError::Invalid(
                "a read follows the stream's detach".to_owned(),
            ));
        };
        frame_read(relay.next_client_chunk().await)
    }

    fn arm(&self) {
        let args = Arc::clone(&self.callback_args);
        self.slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .deferred_callback = Some(Box::new(move |parked: Vec<LoggingFuture>| {
            args.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(parked.len());
            Box::pin(await_parked(parked))
        }));
    }

    async fn wait(&self, until: WaitUntil) {
        match until {
            WaitUntil::Idle => quiesce(&self.gate.state).await,
            WaitUntil::Parked => {
                let mut viewer = self.state.recorded.subscribe();
                drop(viewer.wait_for(|value| *value).await);
            }
            WaitUntil::Hook => {
                let mut viewer = self.state.fired.subscribe();
                drop(viewer.wait_for(|value| *value).await);
            }
        }
    }

    fn observe(&self) -> Result<StreamObservation, SdkHttpError> {
        let fields = self
            .slots
            .fields
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let armed = Flag(fields.deferred_callback.is_some());
        let parked = Flag(fields.deferred_args.is_some());
        drop(fields);
        let failures = drain(&self.state.failures)
            .into_iter()
            .map(|call| {
                Ok(FailureCall {
                    kind: call.failure.kind,
                    message: call.failure.message,
                    endpoint_anthropic: Flag(call.endpoint == EndpointType::Anthropic),
                    request_body: call.request_body,
                    raw_frames: text_frames(call.raw_bytes)?,
                })
            })
            .collect::<Result<Vec<_>, SdkHttpError>>()?;
        let billed = drain(&self.state.recorder.routed)
            .into_iter()
            .map(|call| text_frames(call.raw_bytes))
            .collect::<Result<Vec<_>, SdkHttpError>>()?;
        Ok(StreamObservation {
            billed,
            failures,
            hook_calls: drain(&self.state.hook_calls),
            callback_args: drain(&self.callback_args),
            ran: self.state.recorder.ran.load(Ordering::SeqCst),
            armed,
            parked,
            closed: Flag(self.gate.closed.load(Ordering::SeqCst)),
        })
    }
}

// A relay read becomes the frame the oracle records: a chunk, the end, or the provider's error
fn frame_read(
    chunk: Result<Option<Vec<u8>>, Arc<ReportedFailure>>,
) -> Result<FrameRead, SdkHttpError> {
    Ok(match chunk {
        Ok(Some(bytes)) => FrameRead {
            chunk: Some(String::from_utf8(bytes).map_err(SdkHttpError::FrameUtf8)?),
            end: Flag(false),
            error: None,
        },
        Ok(None) => FrameRead {
            chunk: None,
            end: Flag(true),
            error: None,
        },
        Err(error) => FrameRead {
            chunk: None,
            end: Flag(false),
            error: Some(error.as_ref().to_string()),
        },
    })
}

async fn gate_source(
    http: &Http<'_>,
    call: &MessagesNativeStreamCall,
    gate: &Arc<Gate>,
) -> Result<GateSource, SdkHttpError> {
    Ok(GateSource {
        frames: http.frames(&call.body).await?,
        index: 0,
        gate: Arc::clone(gate),
    })
}

fn install_failure_hook(slots: &StreamSlots, state: &Arc<StreamState>) {
    // binding the concrete Arc first makes Some() the coercion site for the trait object
    let hook = Arc::clone(state);
    slots
        .fields
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .detached_failure_hook = Some(hook);
}

fn relay_setup(
    call: &MessagesNativeStreamCall,
    state: &Arc<StreamState>,
    worker: &Arc<LoggingWorker<LoggingJob>>,
    slots: &Arc<StreamSlots>,
    registry: Arc<StreamRegistry>,
) -> Result<RelaySetup<StreamState, StreamState>, SdkHttpError> {
    Ok(RelaySetup {
        logging: StreamDeps {
            seams: Arc::clone(state),
            worker: Arc::clone(worker),
            slots: Arc::clone(slots),
            clock: Arc::new(StepClock::default()),
            success_handler: Arc::new(PassThroughEndpointLogging {
                injected_log_dispatch: None,
            }),
        },
        logging_obj: logging_object()?,
        request_body: call.body.clone(),
        failure_logging: Arc::clone(state),
        registry,
        limits: RelayLimits {
            queue_maxsize: call.queue_maxsize,
            max_detached_stream_drains: call.max_detached_stream_drains,
        },
        boundaries: ApiLimits::default(),
    })
}

fn text_frames(raw: Vec<Vec<u8>>) -> Result<Vec<String>, SdkHttpError> {
    raw.into_iter()
        .map(|frame| String::from_utf8(frame).map_err(SdkHttpError::FrameUtf8))
        .collect()
}
