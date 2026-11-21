#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::str;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailSpanSink,
};
use betterlite_logging::prometheus_latency::guardrail_series::{
    GuardrailObservation, GuardrailSeries, GuardrailStatus,
};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;
use tokio::time::sleep;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use crate::guardrails::callback_caps::errors::{GuardrailGateError, ProxyHookError};
use crate::guardrails::callback_caps::traits::{
    GuardrailCallback, GuardrailEvent, HeaderHooks, HookOverrides, PostCallHooks, PreCallReturn,
    ProxyCallback, RequestHooks, StreamingHooks,
};
use crate::guardrails::hook_execution::execute::{
    BalanceError, CallContext, GuardrailBalancer, GuardrailBases, HookExecution, UnifiedHooks,
};
use crate::guardrails::hook_execution::metrics::GuardrailMetrics;
use crate::guardrails::hook_execution::sensitive_route::SensitiveRouting;
use crate::guardrails::unified_guardrail::apply_record::InvocationClock;
use crate::guardrails::unified_guardrail::recorder::MetadataGuardrailRecorder;

pub(crate) const CLASS: &str = "TestGuardrail";

#[derive(Default)]
pub(crate) struct CallLog(Mutex<Vec<String>>);

impl CallLog {
    pub(crate) fn push(&self, entry: String) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Answer {
    Pass,
    // writes "edited" into the data it was given
    Edit,
    Replace(JsonMap),
    Text(String),
    Fail(ProxyHookError),
}

pub(crate) struct FakeGuardrail {
    pub(crate) name: Option<&'static str>,
    pub(crate) scan_raw_request: bool,
    pub(crate) native_during_call: bool,
    pub(crate) event_hook_set: bool,
    pub(crate) moderation_check: Option<&'static str>,
    pub(crate) gate: Result<bool, GuardrailGateError>,
    pub(crate) events: Mutex<Vec<GuardrailEvent>>,
}

pub(crate) fn guardrail(name: Option<&'static str>) -> FakeGuardrail {
    FakeGuardrail {
        name,
        scan_raw_request: false,
        native_during_call: false,
        event_hook_set: true,
        moderation_check: None,
        gate: Ok(true),
        events: Mutex::new(Vec::new()),
    }
}

impl GuardrailCallback for FakeGuardrail {
    fn guardrail_name(&self) -> Option<&str> {
        self.name
    }

    fn run_in_parallel(&self) -> bool {
        false
    }

    fn scan_raw_request(&self) -> bool {
        self.scan_raw_request
    }

    fn use_native_during_call_hook(&self) -> bool {
        self.native_during_call
    }

    fn event_hook_is_set(&self) -> bool {
        self.event_hook_set
    }

    fn moderation_check(&self) -> Option<&str> {
        self.moderation_check
    }

    fn should_run_guardrail(
        &self,
        _data: &JsonMap,
        event: GuardrailEvent,
    ) -> Result<bool, GuardrailGateError> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
        self.gate.clone()
    }

    // B3666's marker: a list in the metadata bucket
    fn mark_pre_call_hook_ran(&self, data: &mut JsonMap) {
        let marker = JsonValue::String(format!("ran:{}", self.name.unwrap_or("None")));
        let bucket = data
            .entry("metadata")
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
        let Some(fields) = bucket.as_object_mut() else {
            return;
        };
        let list = fields
            .entry("_pre_call_executed_guardrails")
            .or_insert_with(|| JsonValue::Array(Vec::new()));
        if let Some(items) = list.as_array_mut()
            && !items.contains(&marker)
        {
            items.push(marker);
        }
    }
}

pub(crate) struct Fake {
    pub(crate) class_name: &'static str,
    pub(crate) guardrail: Option<FakeGuardrail>,
    pub(crate) apply_guardrail: bool,
    pub(crate) moderation_override: bool,
    pub(crate) native_lifecycle: bool,
    pub(crate) answer: Answer,
    pub(crate) delay: Duration,
    pub(crate) log: Arc<CallLog>,
    pub(crate) seen: Mutex<Vec<JsonMap>>,
}

pub(crate) fn fake(
    class_name: &'static str,
    guardrail: Option<FakeGuardrail>,
    answer: Answer,
    log: &Arc<CallLog>,
) -> Fake {
    Fake {
        class_name,
        guardrail,
        apply_guardrail: false,
        moderation_override: false,
        native_lifecycle: false,
        answer,
        delay: Duration::ZERO,
        log: Arc::clone(log),
        seen: Mutex::new(Vec::new()),
    }
}

impl Fake {
    async fn act(&self, hook: &str, data: &mut JsonMap) -> Result<PreCallReturn, ProxyHookError> {
        sleep(self.delay).await;
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(data.clone());
        self.log.push(format!("{}:{hook}", self.class_name));
        match &self.answer {
            Answer::Pass => Ok(PreCallReturn::Unchanged),
            Answer::Edit => {
                data.insert(
                    "edited".to_owned(),
                    JsonValue::String(self.class_name.to_owned()),
                );
                Ok(PreCallReturn::Unchanged)
            }
            Answer::Replace(replacement) => Ok(PreCallReturn::Data(replacement.clone())),
            Answer::Text(text) => Ok(PreCallReturn::Text(text.clone())),
            Answer::Fail(error) => Err(error.clone()),
        }
    }

    pub(crate) fn seen(&self) -> Vec<JsonMap> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ProxyCallback for Fake {
    fn overrides(&self) -> HookOverrides {
        HookOverrides {
            apply_guardrail: self.apply_guardrail,
            filter_listed_models: false,
            request: RequestHooks {
                pre_call: true,
                moderation: self.moderation_override,
            },
            post_call: PostCallHooks::default(),
            headers: HeaderHooks::default(),
            streaming: StreamingHooks::default(),
        }
    }

    fn class_name(&self) -> &str {
        self.class_name
    }

    fn is_base_logger(&self) -> bool {
        false
    }

    fn enforces_request_content(&self) -> bool {
        false
    }

    fn use_native_lifecycle_hooks(&self) -> bool {
        self.native_lifecycle
    }

    fn guardrail(&self) -> Option<&dyn GuardrailCallback> {
        self.guardrail
            .as_ref()
            .map(|held| -> &dyn GuardrailCallback { held })
    }

    async fn pre_call(
        &self,
        _user_api_key_dict: &UserAPIKeyAuth,
        data: &mut JsonMap,
        _call_type: CallType,
    ) -> Result<PreCallReturn, ProxyHookError> {
        self.act("pre_call", data).await
    }

    async fn moderation(
        &self,
        data: &mut JsonMap,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _call_type: CallType,
    ) -> Result<(), ProxyHookError> {
        self.act("moderation", data).await.map(|_answer| ())
    }

    async fn post_call_success(
        &self,
        data: &mut JsonMap,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        _response: JsonValue,
    ) -> Result<Option<JsonValue>, ProxyHookError> {
        self.act("post_call", data).await?;
        Ok(Some(JsonValue::String("checked".to_owned())))
    }
}

pub(crate) struct FakeUnified {
    pub(crate) log: Arc<CallLog>,
}

#[async_trait]
impl UnifiedHooks for FakeUnified {
    async fn pre_call(
        &self,
        guardrail: &dyn ProxyCallback,
        _data: &mut JsonMap,
        _context: CallContext<'_>,
    ) -> Result<(), ProxyHookError> {
        self.log
            .push(format!("unified:pre_call:{}", guardrail.class_name()));
        Ok(())
    }

    async fn moderation(
        &self,
        guardrail: &dyn ProxyCallback,
        _data: &mut JsonMap,
        _context: CallContext<'_>,
    ) -> Result<(), ProxyHookError> {
        self.log
            .push(format!("unified:moderation:{}", guardrail.class_name()));
        Ok(())
    }

    async fn post_call_success(
        &self,
        guardrail: &dyn ProxyCallback,
        _data: &mut JsonMap,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        response: JsonValue,
    ) -> Result<Option<JsonValue>, ProxyHookError> {
        self.log
            .push(format!("unified:post_call:{}", guardrail.class_name()));
        Ok(Some(response))
    }
}

pub(crate) struct FakeBalancer {
    pub(crate) balanced: bool,
    pub(crate) pick: Option<Arc<dyn ProxyCallback>>,
}

impl GuardrailBalancer for FakeBalancer {
    fn should_use_load_balancing(&self, _guardrail_name: &str) -> bool {
        self.balanced
    }

    fn available_guardrail(
        &self,
        guardrail_name: &str,
    ) -> Result<Arc<dyn ProxyCallback>, BalanceError> {
        self.pick
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| BalanceError::NoCallback {
                guardrail_name: guardrail_name.to_owned(),
            })
    }
}

pub(crate) fn settings(guardrail_name: Option<&str>) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: guardrail_name.map(str::to_owned),
        supported_event_hooks: None,
        event_hook: Some(LitellmParamsMode::One("pre_call".to_owned())),
        default_on: false,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: false,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: GuardrailSwitches::default(),
    }
}

// every guardrail callback is backed by settings named after it, with event_hook pre_call
pub(crate) struct FakeBases;

impl GuardrailBases for FakeBases {
    fn base(&self, callback: &dyn ProxyCallback) -> Option<Arc<GuardrailBase>> {
        let held = callback.guardrail()?;
        Some(Arc::new(GuardrailBase {
            settings: settings(held.guardrail_name()),
            mode: None,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) name: String,
    pub(crate) latency: Decimal,
    pub(crate) status: GuardrailStatus,
    pub(crate) error_type: Option<String>,
    pub(crate) hook_type: String,
}

#[derive(Default)]
pub(crate) struct FakeMetrics(Mutex<Vec<Observed>>);

impl FakeMetrics {
    pub(crate) fn observed(&self) -> Vec<Observed> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GuardrailMetrics for FakeMetrics {
    fn record(&self, observation: &GuardrailObservation<'_>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Observed {
                name: observation.guardrail_name.to_owned(),
                latency: observation.latency_seconds,
                status: observation.status,
                error_type: observation.error_type.map(str::to_owned),
                hook_type: observation.hook_type.to_owned(),
            });
    }
}

#[derive(Default)]
pub(crate) struct FakeSeries(pub(crate) Mutex<Vec<String>>);

impl GuardrailSeries for FakeSeries {
    type Error = Infallible;

    fn observe(&self, labels: &[(&str, &str)], seconds: Decimal) -> Result<(), Self::Error> {
        let names: Vec<&str> = labels.iter().map(|(_key, value)| *value).collect();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{}={seconds}", names.join(",")));
        Ok(())
    }

    fn increment(&self, labels: &[(&str, &str)]) -> Result<(), Self::Error> {
        let names: Vec<&str> = labels.iter().map(|(_key, value)| *value).collect();
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(names.join(","));
        Ok(())
    }
}

#[async_trait]
impl SensitiveRouting for CallLog {
    async fn set_session_routing(
        &self,
        session_id: &str,
        model: &str,
        _user_api_key_dict: Option<&UserAPIKeyAuth>,
        guardrail_name: Option<&str>,
    ) {
        self.push(format!(
            "{session_id}:{model}:{}",
            guardrail_name.unwrap_or("None")
        ));
    }
}

impl GuardrailSpanSink for CallLog {
    fn emit(&self, record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        let name = record.guardrail_name.as_deref().unwrap_or("None");
        self.push(format!("span:{name}"));
        Ok(())
    }
}

// each reading is one second after the one before
#[derive(Default)]
pub(crate) struct StepClock(AtomicU64);

impl InvocationClock for StepClock {
    fn now(&self) -> SystemTime {
        let step = self.0.fetch_add(1, Ordering::SeqCst);
        UNIX_EPOCH
            .checked_add(Duration::from_secs(step))
            .expect("a clock step fits")
    }
}

pub(crate) struct Harness {
    pub(crate) execution: HookExecution,
    pub(crate) log: Arc<CallLog>,
    pub(crate) metrics: Arc<FakeMetrics>,
    pub(crate) routes: Arc<CallLog>,
    pub(crate) spans: Arc<CallLog>,
}

pub(crate) fn harness() -> Harness {
    let log = Arc::new(CallLog::default());
    let metrics = Arc::new(FakeMetrics::default());
    let routes = Arc::new(CallLog::default());
    let spans = Arc::new(CallLog::default());
    let metrics_port: Arc<dyn GuardrailMetrics> = Arc::<FakeMetrics>::clone(&metrics);
    let routing_port: Arc<dyn SensitiveRouting> = Arc::<CallLog>::clone(&routes);
    let execution = HookExecution {
        unified: Arc::new(FakeUnified {
            log: Arc::clone(&log),
        }),
        balancer: Arc::new(FakeBalancer {
            balanced: false,
            pick: None,
        }),
        bases: Arc::new(FakeBases),
        recorder: Arc::new(MetadataGuardrailRecorder),
        metrics: Some(metrics_port),
        sensitive_routing: Some(routing_port),
        sink: Arc::<CallLog>::clone(&spans),
        clock: Arc::new(StepClock::default()),
    };
    Harness {
        execution,
        log,
        metrics,
        routes,
        spans,
    }
}

pub(crate) fn auth() -> UserAPIKeyAuth {
    UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).expect("an empty key builds")
}

pub(crate) fn map(text: &str) -> JsonMap {
    serde_json::from_str(text).expect("the test map is JSON")
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(Level::WARN)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
