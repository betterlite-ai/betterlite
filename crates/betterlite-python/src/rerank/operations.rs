use super::options::{FamilyChoice, family_choice, map_with, optional_params, refusal};
use super::session::RerankSession;
use betterlite_config::sections::{cohere_rerank_settings, jina_infinity_rerank};
use betterlite_core::api::rerank_provider_plan::dispatch::{
    OptionsRequest, Params, RerankOperations, SendRequest, provider_failure, value_failure,
};
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{BaseRerankConfig, RerankLogging};
use betterlite_provider::api::rerank_http::client::ClientSource;
use betterlite_provider::api::rerank_http::error::{ClientError, RerankHttpError};
use betterlite_provider::api::rerank_http::rerank as rerank_http;
use betterlite_provider::api::rerank_http::{RerankCall as HttpCall, RerankDeps, RerankLogContext};
use betterlite_provider::embedding_rerank_providers::rerank::FamilyRerankConfig;
use betterlite_provider::nvidia_nim_rerank::{NvidiaNimRerankSelection, select_rerank_config};
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset, Utc};
use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

// The client the handler would build; a session owns one outbound client for every call.
struct Direct(ProxiedClient);

impl ClientSource for Direct {
    type Client = ProxiedClient;

    #[cfg_attr(test, mutants::skip)]
    fn client(&self) -> Result<Self::Client, ClientError> {
        Ok(self.0.clone())
    }
}

// The Python package registers no callbacks, so a log record goes nowhere.
struct Sink;

impl LogSink for Sink {
    #[cfg_attr(test, mutants::skip)]
    fn emit(&self, _record: LogRecord) {}
}

struct HttpState<'visit> {
    source: &'visit Direct,
    redactor: &'visit Redactor,
    now: DateTime<FixedOffset>,
}

// B1013's mapping and B2454's transport, wired to the providers the SDK can send through.
pub(crate) struct Operations<'call> {
    session: &'call RerankSession,
    cohere: &'call cohere_rerank_settings::Settings,
    host: &'call jina_infinity_rerank::Settings,
    provider: Mutex<Option<String>>,
    provider_ms: AtomicU64,
}

impl<'call> Operations<'call> {
    #[cfg_attr(test, mutants::skip)]
    pub(crate) const fn new(
        session: &'call RerankSession,
        cohere: &'call cohere_rerank_settings::Settings,
        host: &'call jina_infinity_rerank::Settings,
    ) -> Self {
        Self {
            session,
            cohere,
            host,
            provider: Mutex::new(None),
            provider_ms: AtomicU64::new(0),
        }
    }

    // the provider get_llm_provider resolved, as map_options sees it
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn provider(&self) -> Option<String> {
        self.provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // arerank reports the response time its own transport did not spend as the wrapper's overhead
    #[cfg_attr(test, mutants::skip)]
    pub(crate) fn overhead_ms(&self, response_ms: f64) -> f64 {
        f64::from_bits(self.provider_ms.load(Ordering::Relaxed))
            .mul_add(-1.0_f64, response_ms)
            .max(0.0_f64)
    }

    #[cfg_attr(test, mutants::skip)]
    fn family(&self, family_kind: RerankConfig) -> FamilyRerankConfig<'_> {
        FamilyRerankConfig {
            config: family_kind,
            cohere: self.cohere,
            host: self.host,
            environment: &self.session.environment,
        }
    }

    #[cfg_attr(test, mutants::skip)]
    async fn send_config(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<RerankResponse, Box<RawException>> {
        let source = Direct(self.session.client.clone());
        let http = HttpState {
            source: &source,
            redactor: &self.session.redactor,
            now: Utc::now().fixed_offset(),
        };
        let started = Instant::now();
        let outcome = match family_choice(request.config, request.provider) {
            FamilyChoice::Family(kind) => {
                let config = self.family(kind);
                send_with(&config, request, logging, &http).await
            }
            FamilyChoice::Nvidia => match select_rerank_config(request.model) {
                NvidiaNimRerankSelection::Retrieval(selected) => {
                    send_with(&selected, request, logging, &http).await
                }
                NvidiaNimRerankSelection::Ranking(selected) => {
                    send_with(&selected, request, logging, &http).await
                }
            },
            FamilyChoice::Refused(refusal_kind) => Err(Box::new(refusal(
                request.model,
                request.provider,
                refusal_kind,
            ))),
        };
        self.provider_ms
            .store(started.elapsed().as_secs_f64().to_bits(), Ordering::Relaxed);
        outcome
    }
}

impl RerankOperations for Operations<'_> {
    #[cfg_attr(test, mutants::skip)]
    fn map_options(&self, request: OptionsRequest<'_>) -> Result<Params, Box<RawException>> {
        let (params, config, model, provider) = optional_params(request);
        *self.provider.lock().unwrap_or_else(PoisonError::into_inner) = Some(provider.to_owned());
        match family_choice(config, provider) {
            FamilyChoice::Family(kind) => map_with(&self.family(kind), params, model, provider),
            FamilyChoice::Nvidia => match select_rerank_config(model) {
                NvidiaNimRerankSelection::Retrieval(selected) => {
                    map_with(&selected, params, model, provider)
                }
                NvidiaNimRerankSelection::Ranking(selected) => {
                    map_with(&selected, params, model, provider)
                }
            },
            FamilyChoice::Refused(refusal_kind) => {
                Err(Box::new(refusal(model, provider, refusal_kind)))
            }
        }
    }

    #[cfg_attr(test, mutants::skip)]
    fn send(
        &self,
        request: SendRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<RerankResponse, Box<RawException>>> + Send {
        self.send_config(request, logging)
    }
}

#[cfg_attr(test, mutants::skip)]
async fn send_with<C>(
    config: &C,
    request: SendRequest<'_>,
    logging: &mut LoggingState,
    http: &HttpState<'_>,
) -> Result<RerankResponse, Box<RawException>>
where
    C: BaseRerankConfig,
{
    let mut http_logging = RerankLogging::default();
    let call = HttpCall {
        model: request.model,
        optional_params: request.optional_params,
        api_base: request.api_base,
        api_key: request.api_key,
        litellm_params: Some(request.litellm_params),
        headers: Some(request.headers),
        timeout: timeout_seconds(request.timeout_seconds),
        is_async: Json::Bool(request.is_async),
        model_response: RerankResponse::default(),
        logging: &mut http_logging,
    };
    let sink = Sink;
    let env = prepost_env(&sink, http.redactor, http.now);
    let mut log = RerankLogContext {
        state: logging,
        env: &env,
    };
    let deps = RerankDeps {
        config,
        source: http.source,
        supplied: None,
    };
    match rerank_http(call, deps, &mut log).await {
        Ok(response) => Ok(response),
        Err(error) => Err(http_failure(error, request.model, request.provider)),
    }
}

#[cfg_attr(test, mutants::skip)]
fn prepost_env<'visit>(
    sink: &'visit Sink,
    redactor: &'visit Redactor,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'visit> {
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
        now,
    }
}

#[cfg_attr(test, mutants::skip)]
fn http_failure<Failure>(
    error: RerankHttpError<Failure>,
    model: &str,
    provider: &str,
) -> Box<RawException>
where
    Failure: Error + Send + Sync + 'static,
{
    match error {
        RerankHttpError::Provider(raised) => Box::new(provider_failure(
            raised.status_code,
            &raised.message,
            model,
            provider,
            "BaseLLMException",
        )),
        other @ (RerankHttpError::Config(_)
        | RerankHttpError::Client(_)
        | RerankHttpError::Logging(_)
        | RerankHttpError::RequestUrl { .. }
        | RerankHttpError::HeaderName { .. }
        | RerankHttpError::HeaderValue { .. }) => {
            Box::new(value_failure(&other.to_string(), model, provider))
        }
    }
}

#[cfg_attr(test, mutants::skip)]
fn timeout_seconds(value: Option<f64>) -> Option<Duration> {
    match value {
        Some(seconds) if seconds.is_finite() && seconds >= 0.0_f64 => {
            Duration::try_from_secs_f64(seconds).ok()
        }
        Some(_) | None => None,
    }
}
