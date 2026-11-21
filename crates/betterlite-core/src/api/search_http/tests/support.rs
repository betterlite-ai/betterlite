use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use crate::api::search_http::{SearchHttpCall, SearchHttpError, execute_search};
use crate::api::vector_store_http_write::service::LogContext;

pub(super) type Events = Arc<Mutex<Vec<String>>>;

pub(super) fn record(events: &Events, name: &str) {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(name.to_owned());
}

pub(super) fn recorded(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"search","call_type":"search","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
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
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(super) async fn execute<Provider>(
        &mut self,
        call: SearchHttpCall<'_>,
        provider: &Provider,
        pool: &ClientPool,
        network_mock: bool,
    ) -> Result<SearchResponse, SearchHttpError<Provider::Error>>
    where
        Provider: SearchProvider + Sync,
        Provider::Error: 'static,
    {
        let logger = |_details: &Json| -> Result<(), PrePostError> { Ok(()) };
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
            logger_fn: Some(&logger),
            now: self.now,
        };
        let mut context = LogContext {
            state: &mut self.state,
            env: &env,
        };
        execute_search(call, provider, pool, &mut context, network_mock).await
    }

    pub(super) fn logged(&self, name: &str) -> Option<&Json> {
        let Json::Object(details) = self.state.model_call_details.get("additional_args")? else {
            return None;
        };
        details.get(name)
    }

    pub(super) fn input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }
}

pub(super) fn default_tls() -> TlsPolicy {
    TlsPolicy {
        verification: Verification::Enabled,
        ca_file: None,
        certificate: None,
        http2: false,
        ecdh_curve: None,
        user_agent: String::from("betterlite"),
    }
}

pub(super) fn proxy_policy() -> Arc<ProxyPolicy> {
    Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    )
}

pub(super) fn proxied(limits: TransportLimits, policy: &Arc<ProxyPolicy>) -> ProxiedClient {
    ProxiedClient::build(base_builder().http1_only(), policy, limits)
        .expect("the proxied client builds")
}

pub(super) fn search_call<'call>(
    query: &'call SearchQuery,
    params: &'call Object,
    base: &'call str,
    tls: &'call TlsPolicy,
    routing: &'call Arc<ProxyPolicy>,
) -> SearchHttpCall<'call> {
    SearchHttpCall {
        query,
        optional_params: params,
        timeout: Duration::from_secs(2),
        api_key: None,
        api_base: Some(base.to_owned()),
        headers: Headers::new(),
        client: None,
        tls,
        proxy: routing,
    }
}

#[derive(Debug, Error)]
pub(super) enum TestError {
    #[error("the reply was rejected: {0}")]
    Rejected(String),
    #[error("the reply is not a search response")]
    Decode(#[source] serde_json::Error),
}

#[derive(Clone, Default)]
pub(super) struct Seen {
    pub(super) transform_api_key: Option<String>,
    pub(super) transform_api_base: Option<String>,
    pub(super) transform_headers: Headers,
    pub(super) sign_api_base: String,
    pub(super) sign_api_key: Option<String>,
    pub(super) sign_headers: Headers,
    pub(super) sign_body: Vec<u8>,
    pub(super) sign_params: Object,
}

pub(super) struct TestProvider {
    pub(super) complete: String,
    pub(super) method: HttpMethod,
    pub(super) body: Object,
    pub(super) signed: Headers,
    pub(super) signed_body: Option<Vec<u8>>,
    pub(super) reject: bool,
    pub(super) answer: Option<SearchResponse>,
    pub(super) events: Events,
    pub(super) seen: Mutex<Seen>,
    pub(super) errors: Mutex<Vec<(u16, String, Headers)>>,
}

pub(super) fn test_provider(complete: &str) -> TestProvider {
    TestProvider {
        complete: complete.to_owned(),
        method: HttpMethod::Post,
        body: serde_json::from_str(r#"{"q":"rust"}"#).expect("the body fixture is an object"),
        signed: Headers::new(),
        signed_body: None,
        reject: false,
        answer: None,
        events: Events::default(),
        seen: Mutex::default(),
        errors: Mutex::default(),
    }
}

pub(super) fn seen_values(provider: &TestProvider) -> Seen {
    provider
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn recorded_errors(provider: &TestProvider) -> Vec<(u16, String, Headers)> {
    provider
        .errors
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn provider_error(rejection: &SearchHttpError<TestError>) -> &ProviderError {
    match rejection {
        SearchHttpError::Provider { provider } => provider,
        SearchHttpError::Hook(hook) => panic!("the call ended in a hook failure: {hook}"),
        SearchHttpError::Logging(logging) => {
            panic!("the call ended in a logging failure: {logging}")
        }
    }
}

impl SearchProvider for TestProvider {
    type Body = Object;
    type Error = TestError;

    fn http_method(&self) -> HttpMethod {
        self.method
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        record(&self.events, "validate");
        headers.insert("x-validated".to_owned(), "yes".to_owned());
        headers
    }

    fn complete_url(
        &self,
        _api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Object>,
        _api_key: Option<&SecretString>,
    ) -> Result<String, TestError> {
        record(&self.events, "url");
        Ok(self.complete.clone())
    }

    fn transform_search_request(
        &self,
        _query: &SearchQuery,
        _optional_params: &Object,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
        headers: &Headers,
    ) -> Result<Object, TestError> {
        record(&self.events, "transform");
        let mut observed = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        observed.transform_api_key = api_key.map(|key| key.expose_secret().to_owned());
        observed.transform_api_base = api_base.map(str::to_owned);
        observed.transform_headers = headers.clone();
        drop(observed);
        Ok(self.body.clone())
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Object>,
    ) -> Result<(Headers, Option<Vec<u8>>), TestError> {
        record(&self.events, "sign");
        let mut observed = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        observed.sign_api_base = request.api_base.to_owned();
        observed.sign_api_key = request.api_key.map(|key| key.expose_secret().to_owned());
        observed.sign_headers = headers.clone();
        observed.sign_body = serde_json::to_vec(request.body).expect("the body encodes");
        observed.sign_params = request.optional_params.clone();
        drop(observed);
        let mut signed = headers;
        signed.extend(self.signed.clone());
        Ok((signed, self.signed_body.clone()))
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, TestError> {
        if self.reject {
            return Err(TestError::Rejected(response.text.to_owned()));
        }
        if let Some(answer) = &self.answer {
            return Ok(answer.clone());
        }
        serde_json::from_str(response.text).map_err(TestError::Decode)
    }

    fn http_error(&self, response: &ProviderResponse<'_>) -> ProviderError {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                response.status_code,
                response.text.to_owned(),
                response.headers.clone(),
            ));
        self.error_class(response.text, response.status_code, response.headers)
    }
}
