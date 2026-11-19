use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::file_http_content::config::{
    BufferedResponse, ContentParams, FileContentConfig, Headers, QueryParams,
};
use betterlite_core::api::file_http_content::error::ContentError;
use betterlite_core::api::file_http_content::service::{
    ClientSource, ContentDeps, LogContext, PreparedContent, RetrieveCall, prepare,
};
use betterlite_core::api::vector_store_http_write::error::{BaseLlmException, ClientError};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_provider::api::file_contracts::config::{
    BaseFilesConfig as _, FileLogging, FileParams, FileResponse,
};
use betterlite_provider::manus_files::ManusFilesConfig;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;

use super::super::error::SdkHttpError;

// The pooled client the selected deployment would get; the case env only carries the key and the proxy settings
pub(super) struct Source {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
}

impl ClientSource for Source {
    type Client = ProxiedClient;

    fn client(&self) -> Result<ProxiedClient, ClientError> {
        self.pool
            .client(&self.tls, &self.routing)
            .map_err(ClientError::Pool)
    }
}

pub(super) fn source(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Source, SdkHttpError> {
    let routing = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let tls = TlsPolicy::resolve(config, env, None)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    Ok(Source {
        pool: ClientPool::new(1, TransportLimits::DEFAULT),
        tls,
        routing,
    })
}

// The provider config a Manus content request runs through, with the litellm_params its transforms read
pub(super) struct ProviderContent {
    provider: ManusFilesConfig,
    litellm: FileParams,
}

impl ProviderContent {
    pub(super) fn new(environment: &BTreeMap<String, String>, params: &ContentParams) -> Self {
        let mut litellm = FileParams::new();
        if let Some(api_base) = &params.api_base {
            litellm.insert(String::from("api_base"), Value::String(api_base.clone()));
        }
        if let Some(api_key) = &params.api_key {
            litellm.insert(
                String::from("api_key"),
                Value::String(api_key.expose_secret().to_owned()),
            );
        }
        Self {
            provider: ManusFilesConfig {
                globals: GlobalCredentials::default(),
                secrets: secrets(),
                environment: environment.clone(),
                clock: Box::new(|| 0_i64),
            },
            litellm,
        }
    }
}

impl FileContentConfig for ProviderContent {
    fn transform_content_request(
        &self,
        request: &FileContentRequest,
        _params: &ContentParams,
    ) -> Result<(String, QueryParams), ContentError> {
        let (url, query) = self
            .provider
            .transform_file_content_request(request, &FileParams::new(), &self.litellm)
            .map_err(|error| ContentError::Transform(Box::new(error)))?;
        Ok((url, query_params(query)?))
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        params: &ContentParams,
    ) -> Result<Headers, ContentError> {
        self.provider
            .validate_environment(&mut headers, params.api_key.as_ref())
            .map_err(|error| ContentError::Transform(Box::new(error)))?;
        Ok(headers)
    }

    fn transform_content_response(
        &self,
        response: &BufferedResponse,
        _logging: &mut LoggingState,
        _params: &ContentParams,
    ) -> Result<BinaryContent, ContentError> {
        let raw = FileResponse {
            status_code: response.status_code,
            headers: response.headers.clone(),
            body: response.body.clone(),
            url: Some(response.url.clone()),
        };
        self.provider
            .transform_file_content_response(
                &raw,
                &mut FileLogging {
                    model_call_details: FileParams::new(),
                },
                &self.litellm,
            )
            .map_err(|error| ContentError::Transform(Box::new(error)))
    }

    fn error_class(&self, message: &str, status_code: u16, headers: Headers) -> BaseLlmException {
        let provider = ManusFilesConfig::get_error_class(message, status_code, headers);
        BaseLlmException {
            status_code: provider.status_code,
            message: provider.message,
            headers: provider
                .headers
                .into_iter()
                .map(|(name, value)| (name, Json::Text(value)))
                .collect(),
            status_code_is_synthesized: false,
        }
    }
}

#[derive(Debug, Error)]
#[error("the provider query parameter {name} is not text")]
struct QueryNotText {
    name: String,
}

// LiteLLM's transform returns a dict, so every value it can carry is text
fn query_params(params: FileParams) -> Result<QueryParams, ContentError> {
    let mut query = QueryParams::new();
    for (name, value) in params {
        let Value::String(text) = value else {
            return Err(ContentError::Transform(Box::new(QueryNotText { name })));
        };
        query.insert(name, text);
    }
    Ok(query)
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the oracle makes no OIDC network call"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the oracle has no Azure token provider"),
        })
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call a production request runs: the same state a request would log through, with nothing attached to it
pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    pub(super) state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new(call_type: &str) -> Result<Self, SdkHttpError> {
        let text = format!(
            r#"{{"model":"file-content","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        );
        let init: LoggingInit = serde_json::from_str(&text)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(SdkHttpError::LoggingState)?;
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
        })
    }

    pub(super) fn prepare<'deps, C, S>(
        &mut self,
        deps: &'deps ContentDeps<'deps, C, S>,
        call: &RetrieveCall<'_>,
    ) -> Result<PreparedContent<'deps, C, S>, SdkHttpError>
    where
        C: FileContentConfig,
        S: ClientSource,
    {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        prepare(
            deps,
            call,
            &mut LogContext {
                state: &mut self.state,
                env: &env,
            },
        )
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
    }
}
