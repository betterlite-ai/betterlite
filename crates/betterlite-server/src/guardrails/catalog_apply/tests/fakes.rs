use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::routes::ApplyGuardrailResponse;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_guardrails::guardrail_registry::callback::GuardrailCallback;

use crate::auth::auth_identity::AuthContext;
use crate::guardrails::callback_caps::errors::{HttpException, ProxyHookError};
use crate::guardrails::catalog_apply::error::CatalogApplyError;
use crate::guardrails::catalog_apply::ports::{
    ApplyHooks, ApplyLog, ApplyLogError, Body, GuardrailRunner, PreCallOutcome, SuccessPayload,
};
use crate::registry_read_through::lookup::GuardrailLookup;
use crate::registry_read_through::{Resync, ResyncError};

use super::support::{Event, FailureHook, Handler, LogMode, RunnerMode, Step, SuccessHook};

pub(super) struct FakeRegistry {
    pub(super) callback: GuardrailCallback,
    pub(super) present: Arc<AtomicBool>,
    pub(super) lookups: Arc<AtomicUsize>,
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

pub(super) struct FakeResync {
    pub(super) present: Arc<AtomicBool>,
    pub(super) found: bool,
    pub(super) resyncs: Arc<AtomicUsize>,
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

pub(super) struct FakeRunner {
    pub(super) mode: RunnerMode,
    pub(super) events: Arc<Mutex<Vec<Event>>>,
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

pub(super) struct FakeHooks {
    pub(super) pre_call_reply: Option<Body>,
    pub(super) log: Option<Arc<dyn ApplyLog>>,
    pub(super) success: SuccessHook,
    pub(super) failure: FailureHook,
    pub(super) events: Arc<Mutex<Vec<Event>>>,
}

impl FakeHooks {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    fn success_outcome(
        &self,
        text: String,
        data: &Body,
    ) -> Result<Option<ApplyGuardrailResponse>, ProxyHookError> {
        self.record(Event::Hook {
            step: Step::PostCallSuccess,
            text,
            data: data.clone(),
        });
        match &self.success {
            SuccessHook::Keep => Ok(None),
            SuccessHook::Replace(replacement) => Ok(Some(ApplyGuardrailResponse {
                response_text: replacement.clone(),
            })),
            SuccessHook::Fail(message) => Err(ProxyHookError::Other(message.clone())),
        }
    }
}

#[async_trait]
impl ApplyHooks for FakeHooks {
    async fn pre_call(
        &self,
        data: Body,
        _caller: &AuthContext,
        call_type: &str,
    ) -> Result<PreCallOutcome, ProxyHookError> {
        self.record(Event::Hook {
            step: Step::PreCall,
            text: call_type.to_owned(),
            data: data.clone(),
        });
        let replied = self.pre_call_reply.clone().unwrap_or(data);
        Ok(PreCallOutcome {
            data: replied,
            log: self.log.as_ref().map(Arc::clone),
        })
    }

    async fn post_call_failure(
        &self,
        _caller: &AuthContext,
        error: &CatalogApplyError,
        data: &Body,
    ) -> Result<Option<HttpException>, ProxyHookError> {
        let outcome = self.failure.outcome();
        self.record(Event::Hook {
            step: Step::PostCallFailure,
            text: error.to_string(),
            data: data.clone(),
        });
        outcome
    }

    async fn post_call_success(
        &self,
        data: &Body,
        _caller: &AuthContext,
        response: ApplyGuardrailResponse,
    ) -> Result<Option<ApplyGuardrailResponse>, ProxyHookError> {
        self.success_outcome(response.response_text, data)
    }
}

impl FailureHook {
    fn outcome(&self) -> Result<Option<HttpException>, ProxyHookError> {
        match self {
            Self::Keep => Ok(None),
            Self::Replace { status, detail } => Ok(Some(HttpException {
                status: *status,
                detail: detail.clone(),
            })),
            Self::Fail(message) => Err(ProxyHookError::Other(message.clone())),
        }
    }
}

pub(super) struct FakeLog {
    pub(super) mode: LogMode,
    pub(super) events: Arc<Mutex<Vec<Event>>>,
}

impl FakeLog {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    fn fail_with(
        &self,
        mode: LogMode,
        event: Event,
        error: ApplyLogError,
    ) -> Result<(), ApplyLogError> {
        self.record(event);
        if self.mode == mode {
            return Err(error);
        }
        Ok(())
    }
}

// The sync handlers repeat the async ones with a different event tag; the macro keeps one copy.
macro_rules! log_failure_handler {
    ($method:ident, $handler:ident) => {
        fn $method(
            &self,
            error: &CatalogApplyError,
            _traceback: &str,
        ) -> Result<(), ApplyLogError> {
            self.fail_with(
                LogMode::FailFailureHandlers,
                Event::FailureHandler {
                    handler: Handler::$handler,
                    error: error.to_string(),
                },
                ApplyLogError::Unavailable,
            )
        }
    };
}

macro_rules! log_success_handler {
    ($method:ident, $handler:ident) => {
        fn $method(
            &self,
            payload: SuccessPayload,
            start_time: SystemTime,
            end_time: SystemTime,
            cache_hit: bool,
        ) -> Result<(), ApplyLogError> {
            self.fail_with(
                LogMode::FailSuccessHandlers,
                Event::SuccessHandler {
                    handler: Handler::$handler,
                    payload,
                    start_time,
                    end_time,
                    cache_hit,
                },
                ApplyLogError::Unavailable,
            )
        }
    };
}

#[async_trait]
impl ApplyLog for FakeLog {
    fn mark_pass_through(&self, messages: Vec<OpaqueParams>) -> Result<(), ApplyLogError> {
        self.fail_with(
            LogMode::FailMark,
            Event::MarkPassThrough { messages },
            ApplyLogError::Unavailable,
        )
    }

    async fn async_failure_handler(
        &self,
        error: &CatalogApplyError,
        _traceback: &str,
    ) -> Result<(), ApplyLogError> {
        self.record(Event::FailureHandler {
            handler: Handler::Async,
            error: error.to_string(),
        });
        if self.mode == LogMode::FailFailureHandlers {
            return Err(ApplyLogError::Failed {
                message: "async failure handler".to_owned(),
            });
        }
        Ok(())
    }

    log_failure_handler!(submit_failure_handler, Submit);

    async fn async_success_handler(
        &self,
        payload: SuccessPayload,
        start_time: SystemTime,
        end_time: SystemTime,
        cache_hit: bool,
    ) -> Result<(), ApplyLogError> {
        self.record(Event::SuccessHandler {
            handler: Handler::Async,
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

    log_success_handler!(submit_success_handler, Submit);
}

pub(super) struct FakeSink {
    pub(super) events: Arc<Mutex<Vec<Event>>>,
}

impl GuardrailSpanSink for FakeSink {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        self.events.lock().unwrap().push(Event::SinkRecord {
            record: record.clone(),
        });
        Ok(())
    }
}
