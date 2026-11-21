use std::error::Error as StdError;
use std::time::SystemTime;

use betterlite_logging::prometheus_latency::guardrail_series::{
    GuardrailCollectors, GuardrailObservation, GuardrailSeries, GuardrailStatus,
    record_guardrail_metrics,
};
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;

use super::bridge::{enrich_error, record_raising_guardrail};
use super::execute::{HookAnswer, HookCall, HookExecution, HookPath};
use crate::guardrails::callback_caps::errors::ProxyHookError;
use crate::guardrails::callback_caps::traits::GuardrailCallback;

const NANOS_SCALE: u32 = 9;

// the Prometheus logger's guardrail observation, B3878
pub trait GuardrailMetrics: Send + Sync {
    fn record(&self, observation: &GuardrailObservation<'_>);
}

pub struct PrometheusGuardrails<Latency, Counter> {
    pub latency: Latency,
    pub requests: Counter,
    pub errors: Counter,
}

impl<Latency, Counter, Failure> GuardrailMetrics for PrometheusGuardrails<Latency, Counter>
where
    Latency: GuardrailSeries<Error = Failure> + Send + Sync,
    Counter: GuardrailSeries<Error = Failure> + Send + Sync,
    Failure: StdError + 'static,
{
    fn record(&self, observation: &GuardrailObservation<'_>) {
        let collectors = GuardrailCollectors {
            latency: &self.latency,
            requests: &self.requests,
            errors: &self.errors,
        };
        record_guardrail_metrics(&collectors, observation);
    }
}

// type(e).__name__ of what each variant stands for; Other carries no Python class
const fn error_type_name(error: &ProxyHookError) -> &'static str {
    match error {
        ProxyHookError::Http { .. } => "HTTPException",
        ProxyHookError::Proxy(_) => "ProxyException",
        ProxyHookError::GuardrailRaised { .. } => "GuardrailRaisedException",
        ProxyHookError::SensitiveDataRoute(_) => "SensitiveDataRouteException",
        ProxyHookError::ModifyResponse(_) => "ModifyResponseException",
        ProxyHookError::RejectedRequest { .. } => "RejectedRequestError",
        ProxyHookError::Other(_) => "Exception",
    }
}

pub fn emit_guardrail_metrics(execution: &HookExecution, observation: &GuardrailObservation<'_>) {
    if let Some(metrics) = &execution.metrics {
        metrics.record(observation);
    }
}

// a reroute counts as intervened, any other error as error with its type
pub fn observe<T>(
    execution: &HookExecution,
    guardrail_name: &str,
    started_at: SystemTime,
    outcome: &Result<T, ProxyHookError>,
    hook_type: &str,
) {
    let (status, error_type) = match outcome {
        Ok(_) => (GuardrailStatus::Success, None),
        Err(ProxyHookError::SensitiveDataRoute(_)) => (GuardrailStatus::Intervened, None),
        Err(error) => (GuardrailStatus::Error, Some(error_type_name(error))),
    };
    let elapsed = execution
        .clock
        .now()
        .duration_since(started_at)
        .unwrap_or_default();
    let nanos = i64::try_from(elapsed.as_nanos()).unwrap_or(i64::MAX);
    emit_guardrail_metrics(
        execution,
        &GuardrailObservation {
            guardrail_name,
            latency_seconds: Decimal::new(nanos, NANOS_SCALE),
            status,
            error_type,
            hook_type,
        },
    );
}

pub async fn run_guardrail_with_metrics(
    execution: &HookExecution,
    call: HookCall<'_>,
    path: HookPath,
    data: &mut JsonMap,
) -> Result<HookAnswer, ProxyHookError> {
    let callback = call.callback;
    let hook_type = call.hook.hook_type();
    let started_at = execution.clock.now();
    let mut outcome = execution.run_hook(call, path, data).await;
    if let Err(error) = &mut outcome {
        enrich_error(execution, error, callback);
        record_raising_guardrail(execution, data, callback, error);
    }
    let guardrail_name = callback
        .guardrail()
        .and_then(GuardrailCallback::guardrail_name)
        .unwrap_or(callback.class_name());
    observe(execution, guardrail_name, started_at, &outcome, hook_type);
    outcome
}
