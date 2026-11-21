#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_error::error::ErrorClass;
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
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    ProviderResponse, RequestInput, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::error::SearchFailure;
use super::{SearchConfigLookup, SearchOptions, SearchRequest, SearchServices, asearch};
use crate::api::vector_store_http_write::service::LogContext;

pub(super) const ANSWER: &str = r#"{"results":[{"title":"Rust","url":"https://rust-lang.org","snippet":"A language"}],"took":3}"#;

#[derive(Debug, Error)]
pub(super) enum TestError {
    #[error("the URL hook refused the call")]
    Url,
    #[error("the reply is not a search response")]
    Decode(#[source] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Call {
    Validate {
        headers: Headers,
        api_key: Option<String>,
        api_base: Option<String>,
    },
    Url {
        api_base: Option<String>,
        params: Object,
        has_body: bool,
        api_key: Option<String>,
    },
    Transform {
        query: SearchQuery,
        params: Object,
        headers: Headers,
    },
}

#[derive(Clone, Default)]
pub(super) struct Recorder {
    pub(super) fail_url: bool,
    pub(super) hidden: Object,
    pub(super) calls: Arc<Mutex<Vec<Call>>>,
}

impl Recorder {
    pub(super) fn new(fail_url: bool, hidden: Object) -> Self {
        Self {
            fail_url,
            hidden,
            calls: Arc::default(),
        }
    }

    pub(super) fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: Call) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

impl SearchProvider for Recorder {
    type Body = SearchQuery;
    type Error = TestError;

    fn validate_environment(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.record(Call::Validate {
            headers: headers.clone(),
            api_key: api_key.map(|key| key.expose_secret().to_owned()),
            api_base: api_base.map(str::to_owned),
        });
        headers.insert("x-validated".to_owned(), "yes".to_owned());
        headers
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        body: Option<&SearchQuery>,
        api_key: Option<&SecretString>,
    ) -> Result<String, TestError> {
        self.record(Call::Url {
            api_base: api_base.map(str::to_owned),
            params: optional_params.clone(),
            has_body: body.is_some(),
            api_key: api_key.map(|key| key.expose_secret().to_owned()),
        });
        if self.fail_url {
            return Err(TestError::Url);
        }
        let base = api_base.unwrap_or("http://127.0.0.1:9");
        if base.ends_with("/v1/search") {
            Ok(base.to_owned())
        } else {
            Ok(format!("{base}/v1/search"))
        }
    }

    fn transform_search_request(&self, input: &RequestInput<'_>) -> Result<SearchQuery, TestError> {
        self.record(Call::Transform {
            query: input.query.clone(),
            params: input.optional_params.clone(),
            headers: input.headers.clone(),
        });
        Ok(input.query.clone())
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, TestError> {
        let mut parsed: SearchResponse =
            serde_json::from_str(response.text).map_err(TestError::Decode)?;
        parsed.hidden_params.extend(self.hidden.clone());
        Ok(parsed)
    }
}

pub(super) struct Registry {
    pub(super) config: Recorder,
    pub(super) asked: Mutex<Vec<String>>,
}

impl Registry {
    pub(super) fn new(config: Recorder) -> Self {
        Self {
            config,
            asked: Mutex::default(),
        }
    }

    pub(super) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SearchConfigLookup for Registry {
    type Config = Recorder;

    fn search_config(&self, provider: &str) -> Option<Recorder> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(provider.to_owned());
        (provider == "brave").then(|| self.config.clone())
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Harness {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
    pool: ClientPool,
    tls: TlsPolicy,
    proxy: Arc<ProxyPolicy>,
    pub(super) state: LoggingState,
    pub(super) core: CoreSettings,
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"unset","call_type":"search","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

impl Harness {
    pub(super) fn new() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let state = logging_state(&redactor);
        Self {
            redactor,
            sink: Sink,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
            pool: ClientPool::new(4, TransportLimits::DEFAULT),
            tls: TlsPolicy {
                verification: Verification::Enabled,
                ca_file: None,
                certificate: None,
                http2: false,
                ecdh_curve: None,
                user_agent: String::from("betterlite"),
            },
            proxy: Arc::new(
                ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                    .expect("the proxy policy resolves"),
            ),
            state,
            core: CoreSettings {
                request_timeout_seconds: 5.0_f64,
                request_timeout_explicit: false,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            },
        }
    }

    pub(super) async fn search(
        &mut self,
        request: &SearchRequest,
        registry: &Registry,
    ) -> Result<SearchResponse, SearchFailure<TestError>> {
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
        let services = SearchServices {
            lookup: registry,
            pool: &self.pool,
            tls: &self.tls,
            proxy: &self.proxy,
            core: &self.core,
            network_mock: false,
        };
        let mut context = LogContext {
            state: &mut self.state,
            env: &env,
        };
        asearch(request, &services, &mut context).await
    }
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture parses")
}

pub(super) fn kwargs(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the kwargs fixture parses")
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture parses")
}

pub(super) fn request(query: &str, api_base: &str) -> SearchRequest {
    SearchRequest {
        query: json(query),
        search_provider: "brave".to_owned(),
        options: SearchOptions::default(),
        api_key: Some(SecretString::from("sk-search")),
        api_base: Some(api_base.to_owned()),
        timeout: None,
        extra_headers: Headers::new(),
        kwargs: IndexMap::new(),
    }
}

pub(super) fn status_of(class: &ErrorClass) -> u16 {
    class.metadata().status_code
}
