use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body as HttpBody;
use axum::http::Request;
use axum::response::Response;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType};
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::ApplyGuardrailResponse;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::guardrail_registry::callback::{
    CallbackScoping, CallbackSpec, GuardrailCallback,
};
use betterlite_guardrails::guardrail_registry::integrations::CompressionKind;
use tempfile::TempDir;
use tokio_util::task::TaskTracker;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError};
use crate::guardrails::catalog_apply::CatalogApplyState;
use crate::guardrails::catalog_apply::error::CatalogApplyError;
use crate::guardrails::catalog_apply::ports::{
    ApplyHooks, ApplyLog, ApplyLogError, ApplyPorts, Body, GuardrailRunner, PreCallOutcome,
    SuccessPayload,
};
use crate::guardrails::catalog_apply::routes;
use crate::registry_read_through::lookup::GuardrailLookup;
use crate::registry_read_through::{RegistryReadThrough, Resync, ResyncError};

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
        let log: Option<Arc<dyn ApplyLog>> = self.log.map(|mode| {
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

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Event {
    PreCall {
        call_type: String,
        data: Body,
    },
    MarkPassThrough {
        messages: Vec<OpaqueParams>,
    },
    RunnerCall {
        input_type: GuardrailInputType,
        inputs: GuardrailApiInputs,
        request_data: Body,
        at: SystemTime,
    },
    PostCallFailure {
        error: String,
        data: Body,
    },
    PostCallSuccess {
        response_text: String,
        data: Body,
    },
    AsyncFailure {
        error: String,
    },
    SubmitFailure {
        error: String,
    },
    AsyncSuccess {
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    },
    SubmitSuccess {
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
            Self::PreCall { .. } => "pre_call",
            Self::MarkPassThrough { .. } => "mark_pass_through",
            Self::RunnerCall { .. } => "runner_call",
            Self::PostCallFailure { .. } => "post_call_failure",
            Self::PostCallSuccess { .. } => "post_call_success",
            Self::AsyncFailure { .. } => "async_failure",
            Self::SubmitFailure { .. } => "submit_failure",
            Self::AsyncSuccess { .. } => "async_success",
            Self::SubmitSuccess { .. } => "submit_success",
            Self::SinkRecord { .. } => "sink_record",
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

pub(super) fn runner_call(events: &[Event]) -> RunnerCall {
    let calls: Vec<RunnerCall> = events
        .iter()
        .filter_map(|event| {
            if let Event::RunnerCall {
                input_type,
                inputs,
                request_data,
                at,
            } = event
            {
                Some(RunnerCall {
                    input_type: *input_type,
                    inputs: inputs.clone(),
                    request_data: request_data.clone(),
                    at: *at,
                })
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls.len(), 1, "one guardrail call per request");
    calls[0].clone()
}

pub(super) fn pre_call(events: &[Event]) -> (String, Body) {
    let found: Vec<(String, Body)> = events
        .iter()
        .filter_map(|event| {
            if let Event::PreCall { call_type, data } = event {
                Some((call_type.clone(), data.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(found.len(), 1, "one pre-call hook call per request");
    found[0].clone()
}

pub(super) fn post_call_failure(events: &[Event]) -> (String, Body) {
    let found: Vec<(String, Body)> = events
        .iter()
        .filter_map(|event| {
            if let Event::PostCallFailure { error, data } = event {
                Some((error.clone(), data.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(found.len(), 1, "one failure hook call per failed request");
    found[0].clone()
}

pub(super) fn post_call_success(events: &[Event]) -> (String, Body) {
    let found: Vec<(String, Body)> = events
        .iter()
        .filter_map(|event| {
            if let Event::PostCallSuccess {
                response_text,
                data,
            } = event
            {
                Some((response_text.clone(), data.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one success hook call per successful request"
    );
    found[0].clone()
}

pub(super) fn async_failure(events: &[Event]) -> String {
    let found: Vec<String> = events
        .iter()
        .filter_map(|event| {
            if let Event::AsyncFailure { error } = event {
                Some(error.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(found.len(), 1, "one async failure handler call");
    found[0].clone()
}

pub(super) fn submit_failure(events: &[Event]) -> String {
    let found: Vec<String> = events
        .iter()
        .filter_map(|event| {
            if let Event::SubmitFailure { error } = event {
                Some(error.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(found.len(), 1, "one sync failure handler submit");
    found[0].clone()
}

pub(super) fn marked_messages(events: &[Event]) -> Vec<OpaqueParams> {
    let found: Vec<Vec<OpaqueParams>> = events
        .iter()
        .filter_map(|event| {
            if let Event::MarkPassThrough { messages } = event {
                Some(messages.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(found.len(), 1, "one pass-through patch per logged request");
    found[0].clone()
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct SuccessCall {
    pub(super) payload: SuccessPayload,
    pub(super) start_time: SystemTime,
    pub(super) end_time: SystemTime,
    pub(super) cache_hit: bool,
}

pub(super) fn sink_record(events: &[Event]) -> GuardrailInvocationRecord {
    let records: Vec<GuardrailInvocationRecord> = events
        .iter()
        .filter_map(|event| {
            if let Event::SinkRecord { record } = event {
                Some(record.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(records.len(), 1, "one record per apply call");
    records[0].clone()
}

pub(super) fn async_success(events: &[Event]) -> SuccessCall {
    let calls: Vec<SuccessCall> = events
        .iter()
        .filter_map(|event| {
            if let Event::AsyncSuccess {
                payload,
                start_time,
                end_time,
                cache_hit,
            } = event
            {
                Some(SuccessCall {
                    payload: payload.clone(),
                    start_time: *start_time,
                    end_time: *end_time,
                    cache_hit: *cache_hit,
                })
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls.len(), 1, "one async success handler call");
    calls[0].clone()
}

pub(super) fn submit_success(events: &[Event]) -> SuccessCall {
    let calls: Vec<SuccessCall> = events
        .iter()
        .filter_map(|event| {
            if let Event::SubmitSuccess {
                payload,
                start_time,
                end_time,
                cache_hit,
            } = event
            {
                Some(SuccessCall {
                    payload: payload.clone(),
                    start_time: *start_time,
                    end_time: *end_time,
                    cache_hit: *cache_hit,
                })
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls.len(), 1, "one sync success handler submit");
    calls[0].clone()
}

pub(super) fn caller() -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: "virtual-key".to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles: Vec::new(),
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) fn request(
    method: &str,
    uri: &str,
    body: &str,
    authenticated: bool,
) -> Request<HttpBody> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(HttpBody::from(body.to_owned()))
        .unwrap();
    if authenticated {
        request.extensions_mut().insert(caller());
    }
    request
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

struct FakeRegistry {
    callback: GuardrailCallback,
    present: Arc<AtomicBool>,
    lookups: Arc<AtomicUsize>,
}

impl GuardrailLookup for FakeRegistry {
    fn initialized(&self, guardrail_name: &str) -> Option<&GuardrailCallback> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        if !self.present.load(Ordering::SeqCst) {
            return None;
        }
        let named = self.callback.base.settings.guardrail_name.as_deref() == Some(guardrail_name);
        named.then_some(&self.callback)
    }
}

struct FakeResync {
    present: Arc<AtomicBool>,
    found: bool,
    resyncs: Arc<AtomicUsize>,
}

impl Resync for FakeResync {
    fn resync(&self, _key: &str) -> impl Future<Output = Result<bool, ResyncError>> + Send {
        self.resyncs.fetch_add(1, Ordering::SeqCst);
        if self.found {
            self.present.store(true, Ordering::SeqCst);
        }
        ready(Ok(self.found))
    }
}

struct FakeRunner {
    mode: RunnerMode,
    events: Arc<Mutex<Vec<Event>>>,
}

#[async_trait]
impl GuardrailRunner for FakeRunner {
    async fn apply_guardrail(
        &self,
        _callback: &GuardrailCallback,
        inputs: GuardrailApiInputs,
        request_data: &mut Body,
        input_type: GuardrailInputType,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        self.events.lock().unwrap().push(Event::RunnerCall {
            input_type,
            inputs: inputs.clone(),
            request_data: request_data.clone(),
            at: SystemTime::now(),
        });
        match &self.mode {
            RunnerMode::Echo => Ok(inputs),
            RunnerMode::Texts(texts) => Ok(GuardrailApiInputs {
                texts: Some(texts.clone()),
                ..inputs
            }),
            RunnerMode::EmptyTexts => Ok(GuardrailApiInputs {
                texts: Some(Vec::new()),
                ..inputs
            }),
            RunnerMode::NoTexts => Ok(GuardrailApiInputs {
                texts: None,
                ..inputs
            }),
            RunnerMode::HttpFailure { status, detail } => Err(GuardrailFailure::Http {
                status: *status,
                detail: detail.clone(),
            }),
            RunnerMode::OtherFailure(message) => Err(GuardrailFailure::Other(message.clone())),
        }
    }
}

struct FakeHooks {
    pre_call_reply: Option<Body>,
    log: Option<Arc<dyn ApplyLog>>,
    success: SuccessHook,
    failure: FailureHook,
    events: Arc<Mutex<Vec<Event>>>,
}

#[async_trait]
impl ApplyHooks for FakeHooks {
    async fn pre_call(
        &self,
        data: Body,
        _caller: &AuthContext,
        call_type: &str,
    ) -> Result<PreCallOutcome, ProxyHookError> {
        self.events.lock().unwrap().push(Event::PreCall {
            call_type: call_type.to_owned(),
            data: data.clone(),
        });
        Ok(PreCallOutcome {
            data: self.pre_call_reply.clone().unwrap_or(data),
            log: self.log.as_ref().map(Arc::clone),
        })
    }

    async fn post_call_failure(
        &self,
        _caller: &AuthContext,
        error: &CatalogApplyError,
        data: &Body,
    ) -> Result<Option<HttpException>, ProxyHookError> {
        self.events.lock().unwrap().push(Event::PostCallFailure {
            error: error.to_string(),
            data: data.clone(),
        });
        match &self.failure {
            FailureHook::Keep => Ok(None),
            FailureHook::Replace { status, detail } => Ok(Some(HttpException {
                status: *status,
                detail: detail.clone(),
            })),
            FailureHook::Fail(message) => Err(ProxyHookError::Other(message.clone())),
        }
    }

    async fn post_call_success(
        &self,
        data: &Body,
        _caller: &AuthContext,
        response: ApplyGuardrailResponse,
    ) -> Result<Option<ApplyGuardrailResponse>, ProxyHookError> {
        self.events.lock().unwrap().push(Event::PostCallSuccess {
            response_text: response.response_text.clone(),
            data: data.clone(),
        });
        match &self.success {
            SuccessHook::Keep => Ok(None),
            SuccessHook::Replace(text) => Ok(Some(ApplyGuardrailResponse {
                response_text: text.clone(),
            })),
            SuccessHook::Fail(message) => Err(ProxyHookError::Other(message.clone())),
        }
    }
}

struct FakeLog {
    mode: LogMode,
    events: Arc<Mutex<Vec<Event>>>,
}

#[async_trait]
impl ApplyLog for FakeLog {
    fn mark_pass_through(&self, messages: Vec<OpaqueParams>) -> Result<(), ApplyLogError> {
        self.events
            .lock()
            .unwrap()
            .push(Event::MarkPassThrough { messages });
        if self.mode == LogMode::FailMark {
            return Err(ApplyLogError::Unavailable);
        }
        Ok(())
    }

    async fn async_failure_handler(
        &self,
        error: &CatalogApplyError,
        _traceback: &str,
    ) -> Result<(), ApplyLogError> {
        self.events.lock().unwrap().push(Event::AsyncFailure {
            error: error.to_string(),
        });
        if self.mode == LogMode::FailFailureHandlers {
            return Err(ApplyLogError::Failed {
                message: "async failure handler".to_owned(),
            });
        }
        Ok(())
    }

    fn submit_failure_handler(
        &self,
        error: &CatalogApplyError,
        _traceback: &str,
    ) -> Result<(), ApplyLogError> {
        self.events.lock().unwrap().push(Event::SubmitFailure {
            error: error.to_string(),
        });
        if self.mode == LogMode::FailFailureHandlers {
            return Err(ApplyLogError::Unavailable);
        }
        Ok(())
    }

    async fn async_success_handler(
        &self,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    ) -> Result<(), ApplyLogError> {
        self.events.lock().unwrap().push(Event::AsyncSuccess {
            payload,
            start_time,
            end_time,
            cache_hit,
        });
        if self.mode == LogMode::FailSuccessHandlers {
            return Err(ApplyLogError::Unavailable);
        }
        Ok(())
    }

    fn submit_success_handler(
        &self,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    ) -> Result<(), ApplyLogError> {
        self.events.lock().unwrap().push(Event::SubmitSuccess {
            payload,
            start_time,
            end_time,
            cache_hit,
        });
        if self.mode == LogMode::FailSuccessHandlers {
            return Err(ApplyLogError::Unavailable);
        }
        Ok(())
    }
}

struct FakeSink {
    events: Arc<Mutex<Vec<Event>>>,
}

impl GuardrailSpanSink for FakeSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.events.lock().unwrap().push(Event::SinkRecord {
            record: record.clone(),
        });
        Ok(())
    }
}
