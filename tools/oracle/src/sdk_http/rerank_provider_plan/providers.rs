use super::super::error::SdkHttpError;
use super::report::Sink;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::file_http_content::service::ClientSource;
use betterlite_core::api::rerank_provider_plan::dispatch::{
    Params, SendRequest, provider_failure, value_failure,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::embedding_rerank_providers::RerankConfig;
use betterlite_error::dispatch::RawException;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, OptionalRerankParams, RerankLogging, get_optional_rerank_params,
};
use betterlite_provider::api::rerank_http::error::RerankHttpError;
use betterlite_provider::api::rerank_http::rerank as rerank_http;
use betterlite_provider::api::rerank_http::{RerankCall as HttpCall, RerankDeps, RerankLogContext};
use betterlite_provider::embedding_rerank_providers::rerank::FamilyRerankConfig;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
// The client the handler would build; the case env carries the proxy settings
pub(super) struct Direct(ProxiedClient);
impl ClientSource for Direct {
    type Client = ProxiedClient;
    fn client(&self) -> Result<Self::Client, ClientError> {
        Ok(self.0.clone())
    }
}
pub(super) fn direct(env: &BTreeMap<String, String>) -> Result<Direct, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let client =
        ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))?;
    Ok(Direct(client))
}
pub(super) struct HttpState<'visit> {
    pub(super) source: &'visit Direct,
    pub(super) redactor: &'visit Redactor,
    pub(super) now: DateTime<FixedOffset>,
}
pub(super) const fn family<'settings>(
    family: RerankConfig,
    config: &'settings Config,
    environment: &'settings (dyn EnvironmentLookup + Send + Sync),
) -> FamilyRerankConfig<'settings> {
    FamilyRerankConfig {
        config: family,
        cohere: &config.sections.cohere_rerank_settings,
        host: &config.sections.jina_infinity_rerank,
        environment,
    }
}
pub(super) fn map_with<C>(
    config: &C,
    request: OptionalRerankParams<'_>,
    model: &str,
    provider: &str,
) -> Result<Params, Box<RawException>>
where
    C: BaseRerankConfig,
{
    get_optional_rerank_params(config, request)
        .map_err(|error| Box::new(value_failure(&error.to_string(), model, provider)))
}
pub(super) fn not_wired(model: &str, provider: &str) -> RawException {
    value_failure(
        &format!("the {provider} rerank handler is not wired into the oracle adapter"),
        model,
        provider,
    )
}
pub(super) async fn send_with<C>(
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
fn timeout_seconds(value: Option<f64>) -> Option<Duration> {
    match value {
        Some(seconds) if seconds.is_finite() && seconds >= 0.0 => {
            Duration::try_from_secs_f64(seconds).ok()
        }
        Some(_) | None => None,
    }
}
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
