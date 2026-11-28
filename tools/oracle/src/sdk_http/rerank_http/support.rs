use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::api::file_http_content::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, Headers, MapCohereRerankParams, Params, ResponseTransform,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::ExposeSecret as _;
use thiserror::Error;

use super::super::error::SdkHttpError;
use super::{PreCallSeen, TransformSeen};

// The pooled client the handler's own branch would build; the case env only carries the key and proxy settings
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

#[derive(Debug, Error)]
pub(super) enum ConfigError {
    #[error("the provider answer is not a rerank record")]
    Body(#[from] serde_json::Error),
}

// The injected config: adds the key header, spells out the body, and keeps what each hook was handed
pub(super) struct RecordingConfig {
    events: Arc<Mutex<Vec<String>>>,
    seen: Mutex<Option<TransformSeen>>,
}

impl RecordingConfig {
    pub(super) const fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            events,
            seen: Mutex::new(None),
        }
    }

    fn push(&self, event: &str) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.to_owned());
    }

    pub(super) fn seen(&self) -> Option<TransformSeen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BaseRerankConfig for RecordingConfig {
    type Error = ConfigError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        self.push("validate");
        let mut headers = Headers::new();
        if let Some(key) = request.api_key {
            headers.insert("x-api-key".to_owned(), key.expose_secret().to_owned());
        }
        Ok(headers)
    }

    fn transform_rerank_request(
        &self,
        _model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        self.push("transform");
        Ok(request_body(optional_rerank_params))
    }

    fn transform_rerank_response(
        &self,
        _model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        self.push("response_transform");
        let raw_body: Json = serde_json::from_slice(&request.raw_response.body)?;
        let seen = TransformSeen {
            model: request.model.to_owned(),
            api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
            request_data: object(request.request_data),
            optional_params: object(request.optional_params),
            litellm_params: object(request.litellm_params),
            raw_status: request.raw_response.status_code,
            raw_body,
        };
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(seen);
        Ok(serde_json::from_slice(&request.raw_response.body)?)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        self.push("url");
        api_base.unwrap_or_default().to_owned()
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        _request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(Params::new())
    }
}

fn request_body(optional: &Params) -> Params {
    let mut body = Params::new();
    body.insert(
        "documents".to_owned(),
        optional
            .get("documents")
            .cloned()
            .unwrap_or_else(|| Json::List(Vec::new())),
    );
    body.insert(
        "query".to_owned(),
        optional
            .get("query")
            .cloned()
            .unwrap_or_else(|| Json::Text(String::new())),
    );
    body
}

fn object(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    )
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The handler's future crosses the send, so the recording callback must be Send too
pub(super) type Logger = dyn Fn(&Json) -> Result<(), PrePostError> + Send + Sync;

// The pre-call a production request runs: the same state, with a logger callback that records its turn
pub(super) struct Fixture {
    pub(super) redactor: Redactor,
    pub(super) sink: Sink,
    pub(super) state: LoggingState,
    pub(super) now: DateTime<FixedOffset>,
    pub(super) logger: Box<Logger>,
}

impl Fixture {
    pub(super) fn new(
        call_type: &str,
        events: Arc<Mutex<Vec<String>>>,
    ) -> Result<Self, SdkHttpError> {
        let text = format!(
            r#"{{"model":"rerank-test","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
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
        let logger = Box::new(move |_: &Json| {
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(String::from("pre_call"));
            Ok(())
        });
        Ok(Self {
            redactor,
            sink: Sink,
            state,
            now,
            logger,
        })
    }

    // The body the pre-call callback sees, through the additional_args LiteLLM stores
    pub(super) fn pre_call(&self) -> Option<PreCallSeen> {
        let input = self.state.model_call_details.get("input")?;
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        let body = details.get("complete_input_dict")?;
        Some(PreCallSeen {
            input: input.clone(),
            body: body.clone(),
        })
    }
}
