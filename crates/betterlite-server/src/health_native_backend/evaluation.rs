use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::decisions::{DecisionsEntryError, DecisionsPrepare};
use betterlite_core::systemone::{SystemOneArgs, SystemOneDeps, asystemone};
use betterlite_http::interactions::base::clients::HttpClients;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::call_helpers::headers::provider_response_headers;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::systemone::DecisionsResponse;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use serde::de::Error as _;

use super::seams::EvaluationProbeCall;
use crate::health_probe_modes::calls::{EvaluationCall, HealthProbeReply};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::evaluation::EvaluationCallParams;
use crate::health_probe_modes::logging::HealthProbeLogging as _;

mod tests;

const TIMEOUT_KEY: &str = "timeout";
const STATE_KEY: &str = "state";
const QUESTIONS_KEY: &str = "questions";

// the evaluation mode's arguments as litellm.asystemone takes them, which is what B4440 already collected
pub fn evaluation_args(params: &EvaluationCallParams) -> Result<SystemOneArgs, HealthProbeError> {
    Ok(SystemOneArgs {
        model: params.model.clone(),
        state: Some(decision_json(&params.state, STATE_KEY)?),
        questions: Some(decision_json(&params.questions, QUESTIONS_KEY)?),
        safety_identifier: None,
        api_key: params.api_key.clone(),
        api_base: params.api_base.clone(),
        timeout: timeout(params.timeout)?,
        custom_llm_provider: params.custom_llm_provider.clone(),
        extra_headers: params
            .extra_headers
            .clone()
            .map(|headers| headers.into_iter().collect()),
        kwargs: params.extra.clone(),
    })
}

// ahealth_check reads _hidden_params.headers off the answer; the entry keeps every provider header it was handed
#[must_use]
pub fn evaluation_reply(response: &DecisionsResponse) -> HealthProbeReply {
    let hidden = Json::Object(response.hidden_params.clone());
    let headers = provider_response_headers(Some(&hidden)).unwrap_or_default();
    HealthProbeReply {
        hidden_headers: headers.into_iter().collect(),
    }
}

// what one evaluation call takes from the server: its request resolution, its globals and its drop-params switch
pub trait EvaluationSetup: Send + Sync {
    fn resolution(&self) -> ResolutionContext<'_>;
    fn globals(&self) -> LiteLlmGlobals;
    fn azure_ai(&self) -> AzureAiConfig;
    fn global_drop_params(&self) -> bool;
}

pub struct EvaluationProbe<C, S> {
    pub clients: C,
    pub setup: S,
    pub redactor: Arc<Redactor>,
    pub clock: fn() -> DateTime<FixedOffset>,
}

#[async_trait]
impl<C, S> EvaluationProbeCall for EvaluationProbe<C, S>
where
    C: Fn() -> HttpClients + Send + Sync,
    S: EvaluationSetup,
{
    async fn evaluate(&self, call: EvaluationCall) -> Result<HealthProbeReply, HealthProbeError> {
        let args = evaluation_args(&call.params)?;
        let logging = call
            .logging
            .as_ref()
            .and_then(|logging| logging.logging_state());
        let resolution = self.setup.resolution();
        let sink = TracingSink;
        let env = logging_environment(self.redactor.as_ref(), &sink, self.clock);
        let mut clients = (self.clients)();
        let output = asystemone(
            SystemOneDeps {
                clients: &mut clients,
                env: &env,
                prepare: DecisionsPrepare {
                    resolution: &resolution,
                    globals: self.setup.globals(),
                    azure_ai: self.setup.azure_ai(),
                    logging: logging.as_deref(),
                    global_drop_params: self.setup.global_drop_params(),
                },
            },
            args,
        )
        .await
        .map_err(entry_failure)?;
        Ok(evaluation_reply(&output))
    }
}

fn decision_json<T>(value: &T, field: &'static str) -> Result<Json, HealthProbeError>
where
    T: Serialize,
{
    let text = serde_json::to_string(value)
        .map_err(|source| HealthProbeError::EvaluationPayload { field, source })?;
    serde_json::from_str(&text)
        .map_err(|source| HealthProbeError::EvaluationPayload { field, source })
}

fn timeout(seconds: Option<f64>) -> Result<Option<Duration>, HealthProbeError> {
    let Some(seconds) = seconds else {
        return Ok(None);
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|source| HealthProbeError::EvaluationPayload {
            field: TIMEOUT_KEY,
            source: serde_json::Error::custom(source.to_string()),
        })
}

fn entry_failure(error: DecisionsEntryError) -> HealthProbeError {
    let message = error.to_string();
    let class = error.class();
    HealthProbeError::Backend {
        class,
        message,
        source: Box::new(error),
    }
}

struct TracingSink;

impl LogSink for TracingSink {
    fn emit(&self, _record: LogRecord) {}
}

fn logging_environment<'env>(
    redactor: &'env Redactor,
    sink: &'env TracingSink,
    clock: fn() -> DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: clock(),
    }
}
