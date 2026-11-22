mod failure;
pub mod ports;
mod tests;

use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionError, ResolutionRequest, ResolvedModel};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use self::ports::{CatalogMode, CatalogModes, ProviderResolver};
use crate::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::logging::{HealthProbeLogging, ProbeLogging};
use crate::health_probe_modes::params::{HealthProbeParams, Param};
use crate::health_probe_modes::plan::{HealthProbeArguments, plan_health_probe};
use crate::health_probe_modes::response::{HealthCheckResponse, health_check_response};
use crate::health_probe_modes::run::run_health_probe;
use crate::health_probe_modes::tracking::add_tracking_information;
use crate::health_probe_modes::wildcard::probe_wildcard_model;

pub const DEFAULT_MODE: &str = "chat";
const NO_CACHE_KEY: &str = "no-cache";

// what a health check answers: the three rate limit headers, or the error result LiteLLM builds in its except block
#[derive(Debug)]
pub enum HealthCheckOutcome {
    Healthy(HealthCheckResponse),
    // boxed: the failure record is over 200 bytes larger than the three-header response
    Failed(Box<HealthCheckFailure>),
}

#[derive(Debug)]
pub struct HealthCheckFailure {
    pub error: String,
    // None for the missing-mode result, which LiteLLM returns without the key at all
    pub raw_request: Option<RawRequestRecord>,
    pub exception: HealthEntryError,
}

// RawRequestTypedDict: what the provider call recorded about its outgoing request
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawRequestRecord {
    pub raw_request_api_base: Option<String>,
    pub raw_request_body: Option<Json>,
    pub raw_request_headers: Option<Json>,
    pub error: Option<String>,
}

// ahealth_check's own failures; each Display is the exact text LiteLLM puts in front of the stack text
#[derive(Debug, Error)]
pub enum HealthEntryError {
    #[error("model not set")]
    ModelNotSet,
    #[error("{source}")]
    Resolution {
        #[source]
        source: ResolutionError,
    },
    #[error("Mode {mode} not supported. See modes here: https://docs.litellm.ai/docs/proxy/health")]
    UnsupportedMode { mode: String },
    #[error("{source}")]
    Probe {
        #[source]
        source: HealthProbeError,
    },
}

// ahealth_check's dependencies, injected: the transport, the model list, the catalog, the resolver and the redaction state
pub struct HealthCheckEntry {
    pub backend: Arc<dyn HealthProbeBackend>,
    pub cheapest: Arc<dyn CheapestChatModels>,
    pub documents: Arc<dyn OcrHealthDocuments>,
    pub resolver: Arc<dyn ProviderResolver>,
    pub catalog: Arc<dyn CatalogModes>,
    pub redactor: Arc<Redactor>,
    pub redact_secrets: bool,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub next_id: fn() -> String,
}

struct FlowFailure {
    mode: Option<String>,
    error: HealthEntryError,
}

struct PreparedProbe {
    model: String,
    provider: String,
    mode: String,
    params: HealthProbeParams,
}

impl HealthCheckEntry {
    pub async fn check_health(
        &self,
        mut params: HealthProbeParams,
        mode: Option<String>,
        prompt: Option<String>,
        input: Option<Vec<String>>,
    ) -> HealthCheckOutcome {
        let logging: Arc<dyn HealthProbeLogging> = match self.logging() {
            Ok(logging) => Arc::new(logging),
            Err(error) => {
                return HealthCheckOutcome::Failed(Box::new(self.failure(
                    mode.as_deref(),
                    error,
                    None,
                )));
            }
        };
        params.litellm_logging_obj = Some(Arc::clone(&logging));
        let arguments = HealthProbeArguments { prompt, input };
        match self
            .checked(params, mode, arguments, logging.as_ref())
            .await
        {
            Ok(response) => HealthCheckOutcome::Healthy(response),
            Err(failure) => HealthCheckOutcome::Failed(failure),
        }
    }

    // steps 2 to 11: every failure becomes the result LiteLLM's except block builds
    async fn checked(
        &self,
        params: HealthProbeParams,
        mode: Option<String>,
        arguments: HealthProbeArguments,
        logging: &dyn HealthProbeLogging,
    ) -> Result<HealthCheckResponse, Box<HealthCheckFailure>> {
        let tracked = match add_tracking_information(params) {
            Ok(tracked) => tracked,
            Err(error) => {
                let probe = HealthEntryError::Probe { source: error };
                return Err(Box::new(self.failure(
                    mode.as_deref(),
                    probe,
                    Some(logging),
                )));
            }
        };
        let prepared = match self.prepare(tracked, mode) {
            Ok(prepared) => prepared,
            Err(failure) => {
                return Err(Box::new(self.failure(
                    failure.mode.as_deref(),
                    failure.error,
                    Some(logging),
                )));
            }
        };
        match self.execute(prepared, arguments).await {
            Ok(response) => Ok(response),
            Err(failure) => Err(Box::new(self.failure(
                failure.mode.as_deref(),
                failure.error,
                Some(logging),
            ))),
        }
    }

    // steps 3 to 8: the model, the catalog modes, resolution, the no-cache flag and the chat default
    fn prepare(
        &self,
        mut params: HealthProbeParams,
        mut mode: Option<String>,
    ) -> Result<PreparedProbe, FlowFailure> {
        let Some(mut model) = params.model.as_option().cloned() else {
            return Err(FlowFailure {
                mode,
                error: HealthEntryError::ModelNotSet,
            });
        };
        self.apply_catalog_mode(&model, &mut mode);
        let resolved = match self.resolve(&model, &params) {
            Ok(resolved) => resolved,
            Err(error) => {
                return Err(FlowFailure {
                    mode,
                    error: HealthEntryError::Resolution { source: error },
                });
            }
        };
        let ResolvedModel {
            model: resolved_model,
            provider,
            ..
        } = resolved;
        model = resolved_model;
        self.apply_catalog_mode(&model, &mut mode);
        let mut cache = IndexMap::new();
        cache.insert(NO_CACHE_KEY.to_owned(), Json::Bool(true));
        params.cache = Param::Value(Json::Object(cache));
        let mode_name = chat_mode(mode);
        Ok(PreparedProbe {
            model,
            provider: provider.slug().to_owned(),
            mode: mode_name,
            params,
        })
    }

    // steps 9 to 11: the wildcard probe, the mode plan and the header filter
    async fn execute(
        &self,
        prepared: PreparedProbe,
        arguments: HealthProbeArguments,
    ) -> Result<HealthCheckResponse, FlowFailure> {
        if prepared.model.contains('*') {
            return self.probe_wildcard(prepared).await;
        }
        self.probe_mode(prepared, arguments).await
    }

    // step 9: a star model is checked through the provider's three cheapest chat models, with no header step
    async fn probe_wildcard(
        &self,
        prepared: PreparedProbe,
    ) -> Result<HealthCheckResponse, FlowFailure> {
        let PreparedProbe {
            model,
            provider,
            mode,
            params,
        } = prepared;
        probe_wildcard_model(
            &model,
            &provider,
            params,
            self.cheapest.as_ref(),
            self.backend.as_ref(),
        )
        .await
        .map_err(|source| FlowFailure {
            mode: Some(mode),
            error: HealthEntryError::Probe { source },
        })
    }

    // steps 10 and 11: the mode plan, the probe call and the three headers kept from the reply
    async fn probe_mode(
        &self,
        prepared: PreparedProbe,
        arguments: HealthProbeArguments,
    ) -> Result<HealthCheckResponse, FlowFailure> {
        let PreparedProbe {
            model,
            provider,
            mode,
            params,
        } = prepared;
        let Some(plan) = plan_health_probe(&mode, &model, &provider, params, arguments) else {
            return Err(FlowFailure {
                mode: Some(mode.clone()),
                error: HealthEntryError::UnsupportedMode { mode },
            });
        };
        let reply =
            match run_health_probe(plan, self.backend.as_ref(), self.documents.as_ref()).await {
                Ok(reply) => reply,
                Err(source) => {
                    return Err(FlowFailure {
                        mode: Some(mode),
                        error: HealthEntryError::Probe { source },
                    });
                }
            };
        Ok(health_check_response(&reply.hidden_headers))
    }

    fn apply_catalog_mode(&self, model: &str, mode: &mut Option<String>) {
        if mode.is_some() {
            return;
        }
        if let CatalogMode::Mode(found) = self.catalog.catalog_mode(model) {
            *mode = Some(found);
        }
    }

    fn resolve(
        &self,
        model: &str,
        params: &HealthProbeParams,
    ) -> Result<ResolvedModel, ResolutionError> {
        let request = ResolutionRequest {
            model,
            custom_llm_provider: params.custom_llm_provider.as_option().map(String::as_str),
            api_base: params.api_base.as_option().map(String::as_str),
            api_key: params.api_key.as_ref(),
        };
        self.resolver.resolve(&request)
    }

    // step 1: the logging object LiteLLM builds for every health check call
    fn logging(&self) -> Result<ProbeLogging, HealthEntryError> {
        let mut correlation = CorrelationContext::default();
        let setup = CorrelationSetup {
            context: &mut correlation,
            settings: CorrelationSettings::default(),
        };
        let init = LoggingInit {
            model: String::new(),
            call_type: "acompletion".to_owned(),
            start_time: (self.clock)(),
            litellm_call_id: (self.next_id)(),
            function_id: (self.next_id)(),
            messages: Some(Messages::Values(Vec::new())),
            stream: Some(false),
            litellm_trace_id: None,
            kwargs: IndexMap::new(),
            litellm_params: IndexMap::new(),
            callback_dynamic_params: IndexMap::new(),
            trusted_callback_vars: Vec::new(),
            standard_built_in_tools_params: IndexMap::new(),
            dynamic_callbacks: DynamicCallbacks::default(),
            applied_guardrails: None,
            request_logging: RequestLoggingFlags {
                log_raw_request_response: true,
                raw_request_only: false,
            },
            callback_policy: CallbackPolicy::default(),
        };
        LoggingState::new(init, setup, &self.redactor, self.next_id, || Duration::ZERO)
            .map(ProbeLogging::from)
            .map_err(|source| HealthEntryError::Probe {
                source: HealthProbeError::Logging { source },
            })
    }
}

fn chat_mode(mode: Option<String>) -> String {
    match mode.as_deref() {
        None | Some("") => DEFAULT_MODE.to_owned(),
        Some(name) => name.to_owned(),
    }
}
