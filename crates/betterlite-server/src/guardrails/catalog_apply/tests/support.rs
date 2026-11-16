use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use axum::Router;
use axum::body::Body as HttpBody;
use axum::http::Request;
use axum::response::Response;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_records::record::GuardrailInvocationRecord;
use betterlite_guardrails::guardrail_registry::callback::{
    CallbackScoping, CallbackSpec, GuardrailCallback,
};
use betterlite_guardrails::guardrail_registry::integrations::CompressionKind;
use tempfile::TempDir;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

use crate::guardrails::catalog_apply::CatalogApplyState;
use crate::guardrails::catalog_apply::ports::{ApplyLog, ApplyPorts, Body, SuccessPayload};
use crate::guardrails::catalog_apply::routes;
use crate::registry_read_through::RegistryReadThrough;

use super::fakes::{FakeHooks, FakeLog, FakeRegistry, FakeResync, FakeRunner, FakeSink};

pub(super) struct World {
    pub(super) events: Arc<Mutex<Vec<Event>>>,
    pub(super) lookups: Arc<AtomicUsize>,
    pub(super) resyncs: Arc<AtomicUsize>,
    pub(super) present: Arc<AtomicBool>,
    pub(super) resync_finds: bool,
    pub(super) runner: RunnerMode,
    pub(super) success_hook: SuccessHook,
    pub(super) failure_hook: FailureHook,
    pub(super) log: Option<LogMode>,
    pub(super) pre_call_reply: Option<Body>,
    pub(super) event_hook: Option<LitellmParamsMode>,
    pub(super) guardrail_name: String,
    pub(super) roots: Arc<[PathBuf]>,
    temp: TempDir,
}

impl World {
    pub(super) fn new() -> Self {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("categories")).unwrap();
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            lookups: Arc::new(AtomicUsize::new(0)),
            resyncs: Arc::new(AtomicUsize::new(0)),
            present: Arc::new(AtomicBool::new(true)),
            resync_finds: false,
            runner: RunnerMode::Echo,
            success_hook: SuccessHook::Keep,
            failure_hook: FailureHook::Keep,
            log: None,
            pre_call_reply: None,
            event_hook: None,
            guardrail_name: "safety".to_owned(),
            roots: Arc::from(vec![temp.path().to_path_buf()]),
            temp,
        }
    }

    pub(super) fn write_category(&self, name: &str, extension: &str, content: &str) {
        fs::write(self.category_path(name, extension), content).unwrap();
    }

    pub(super) fn write_category_bytes(&self, name: &str, extension: &str, content: &[u8]) {
        fs::write(self.category_path(name, extension), content).unwrap();
    }

    pub(super) fn category_path(&self, name: &str, extension: &str) -> PathBuf {
        self.temp
            .path()
            .join("categories")
            .join(format!("{name}.{extension}"))
    }

    pub(super) fn outside_path(&self, name: &str) -> PathBuf {
        self.temp.path().join(name)
    }

    pub(super) fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub(super) async fn send(&self, request: Request<HttpBody>) -> Response {
        self.router().oneshot(request).await.unwrap()
    }

    fn router(&self) -> Router {
        let log: Option<Arc<dyn ApplyLog>> = self.log.map(|mode| -> Arc<dyn ApplyLog> {
            Arc::new(FakeLog {
                mode,
                events: Arc::clone(&self.events),
            })
        });
        let ports = ApplyPorts {
            runner: Arc::new(FakeRunner {
                mode: self.runner.clone(),
                events: Arc::clone(&self.events),
            }),
            hooks: Arc::new(FakeHooks {
                pre_call_reply: self.pre_call_reply.clone(),
                log,
                success: self.success_hook.clone(),
                failure: self.failure_hook.clone(),
                events: Arc::clone(&self.events),
            }),
            sink: Arc::new(FakeSink {
                events: Arc::clone(&self.events),
            }),
        };
        let registry = FakeRegistry {
            callback: callback(&self.guardrail_name, self.event_hook.clone()),
            present: Arc::clone(&self.present),
            lookups: Arc::clone(&self.lookups),
        };
        let read_through = RegistryReadThrough::new(FakeResync {
            present: Arc::clone(&self.present),
            found: self.resync_finds,
            resyncs: Arc::clone(&self.resyncs),
        });
        routes(CatalogApplyState {
            ports: Arc::new(ports),
            lookup: Arc::new(registry),
            read_through: Arc::new(read_through),
            roots: Arc::clone(&self.roots),
            tasks: TaskTracker::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub(super) enum RunnerMode {
    Echo,
    Texts(Vec<String>),
    EmptyTexts,
    NoTexts,
    HttpFailure { status: u16, detail: String },
    OtherFailure(String),
}

#[derive(Debug, Clone)]
pub(super) enum SuccessHook {
    Keep,
    Replace(String),
    Fail(String),
}

#[derive(Debug, Clone)]
pub(super) enum FailureHook {
    Keep,
    Replace { status: u16, detail: String },
    Fail(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum LogMode {
    #[default]
    Passing,
    FailMark,
    FailFailureHandlers,
    FailSuccessHandlers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    PreCall,
    PostCallFailure,
    PostCallSuccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handler {
    Async,
    Submit,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Event {
    MarkPassThrough {
        messages: Vec<OpaqueParams>,
    },
    RunnerCall {
        input_type: GuardrailInputType,
        inputs: GuardrailApiInputs,
        request_data: Body,
        at: SystemTime,
    },
    Hook {
        step: Step,
        text: String,
        data: Body,
    },
    FailureHandler {
        handler: Handler,
        error: String,
    },
    SuccessHandler {
        handler: Handler,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    },
    SinkRecord {
        record: GuardrailInvocationRecord,
    },
}

impl Event {
    pub(super) const fn kind(&self) -> &'static str {
        match self {
            Self::MarkPassThrough { .. } => "mark_pass_through",
            Self::RunnerCall { .. } => "runner_call",
            Self::Hook {
                step: Step::PreCall,
                ..
            } => "pre_call",
            Self::Hook {
                step: Step::PostCallFailure,
                ..
            } => "post_call_failure",
            Self::Hook {
                step: Step::PostCallSuccess,
                ..
            } => "post_call_success",
            Self::FailureHandler {
                handler: Handler::Async,
                ..
            } => "async_failure",
            Self::FailureHandler {
                handler: Handler::Submit,
                ..
            } => "submit_failure",
            Self::SuccessHandler {
                handler: Handler::Async,
                ..
            } => "async_success",
            Self::SuccessHandler {
                handler: Handler::Submit,
                ..
            } => "submit_success",
            Self::SinkRecord { .. } => "sink_record",
        }
    }

    fn runner_call(&self) -> Option<RunnerCall> {
        let Self::RunnerCall {
            input_type,
            inputs,
            request_data,
            at,
        } = self
        else {
            return None;
        };
        Some(RunnerCall {
            input_type: *input_type,
            inputs: inputs.clone(),
            request_data: request_data.clone(),
            at: *at,
        })
    }

    fn hook(&self, step: Step) -> Option<(String, Body)> {
        let Self::Hook {
            step: recorded,
            text,
            data,
        } = self
        else {
            return None;
        };
        (*recorded == step).then(|| (text.clone(), data.clone()))
    }

    fn failure(&self, handler: Handler) -> Option<String> {
        let Self::FailureHandler {
            handler: recorded,
            error,
        } = self
        else {
            return None;
        };
        (*recorded == handler).then(|| error.clone())
    }

    fn success(&self, handler: Handler) -> Option<SuccessCall> {
        match self {
            Self::SuccessHandler {
                handler: recorded,
                payload,
                start_time,
                end_time,
                cache_hit,
            } if *recorded == handler => Some(SuccessCall {
                payload: payload.clone(),
                start_time: *start_time,
                end_time: *end_time,
                cache_hit: *cache_hit,
            }),
            _ => None,
        }
    }

    fn marked_messages(&self) -> Option<Vec<OpaqueParams>> {
        match self {
            Self::MarkPassThrough { messages } => Some(messages.clone()),
            _ => None,
        }
    }

    fn sink_record(&self) -> Option<GuardrailInvocationRecord> {
        match self {
            Self::SinkRecord { record } => Some(record.clone()),
            _ => None,
        }
    }
}

pub(super) fn kinds(events: &[Event]) -> Vec<&'static str> {
    events.iter().map(Event::kind).collect()
}

#[derive(Debug, Clone)]
pub(super) struct RunnerCall {
    pub(super) input_type: GuardrailInputType,
    pub(super) inputs: GuardrailApiInputs,
    pub(super) request_data: Body,
    pub(super) at: SystemTime,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SuccessCall {
    pub(super) payload: SuccessPayload,
    pub(super) start_time: SystemTime,
    pub(super) end_time: SystemTime,
    pub(super) cache_hit: bool,
}

fn single<T, F>(events: &[Event], pick: F, what: &str) -> T
where
    F: Fn(&Event) -> Option<T>,
{
    let found: Vec<T> = events.iter().filter_map(pick).collect();
    assert_eq!(found.len(), 1, "{what}");
    found.swap_remove(0)
}

pub(super) fn runner_call(events: &[Event]) -> RunnerCall {
    single(events, Event::runner_call, "one guardrail call per request")
}

pub(super) fn pre_call(events: &[Event]) -> (String, Body) {
    hook(events, Step::PreCall)
}

pub(super) fn post_call_failure(events: &[Event]) -> (String, Body) {
    hook(events, Step::PostCallFailure)
}

pub(super) fn post_call_success(events: &[Event]) -> (String, Body) {
    hook(events, Step::PostCallSuccess)
}

fn hook(events: &[Event], step: Step) -> (String, Body) {
    single(
        events,
        |event| event.hook(step),
        "one hook call per request",
    )
}

pub(super) fn async_failure(events: &[Event]) -> String {
    failure(events, Handler::Async)
}

pub(super) fn submit_failure(events: &[Event]) -> String {
    failure(events, Handler::Submit)
}

fn failure(events: &[Event], handler: Handler) -> String {
    single(
        events,
        |event| event.failure(handler),
        "one failure handler call",
    )
}

pub(super) fn async_success(events: &[Event]) -> SuccessCall {
    success(events, Handler::Async)
}

pub(super) fn submit_success(events: &[Event]) -> SuccessCall {
    success(events, Handler::Submit)
}

fn success(events: &[Event], handler: Handler) -> SuccessCall {
    single(
        events,
        |event| event.success(handler),
        "one success handler call",
    )
}

pub(super) fn marked_messages(events: &[Event]) -> Vec<OpaqueParams> {
    single(events, Event::marked_messages, "one pass-through patch")
}

pub(super) fn sink_record(events: &[Event]) -> GuardrailInvocationRecord {
    single(events, Event::sink_record, "one record per apply call")
}

pub(super) async fn body_bytes(response: Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec()
}

pub(super) async fn body_value(response: Response) -> GuardrailValue {
    serde_json::from_slice(&body_bytes(response).await).unwrap()
}

fn callback(guardrail_name: &str, event_hook: Option<LitellmParamsMode>) -> GuardrailCallback {
    GuardrailCallback {
        base: GuardrailBase {
            settings: GuardrailSettings {
                guardrail_name: Some(guardrail_name.to_owned()),
                supported_event_hooks: None,
                event_hook,
                default_on: false,
                mask_request_content: false,
                mask_response_content: false,
                violation_message_template: None,
                end_session_after_n_fails: None,
                on_violation: None,
                realtime_violation_message: None,
                on_sensitive_data: None,
                sensitive_data_route_to_model: None,
                sticky_session_routing: true,
                run_in_parallel: false,
                scan_raw_request: false,
                only_scan_new_messages: false,
                timeout: None,
                switches: GuardrailSwitches::default(),
            },
            mode: None,
        },
        spec: CallbackSpec::Compression(CompressionKind::Headroom),
        scoping: CallbackScoping::default(),
    }
}
