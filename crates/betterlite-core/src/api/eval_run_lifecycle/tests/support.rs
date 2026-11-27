use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::request::Request;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::evals::error::EvalsError;
use betterlite_openai::evals::openai::OpenAiEvalsConfig;
use betterlite_openai::evals::transformation::{
    BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext,
};
use betterlite_openai::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_run_lifecycle::handlers::{EvalRunHandlers, RunHandlerRequest};

pub(crate) const EVAL_ID: &str = "eval_1";
pub(crate) const RUN_ID: &str = "run_1";
pub(crate) const CALL_ID: &str = "call-1";
pub(crate) const OPENAI_BASE: &str = "https://api.openai.com";
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(6000);

pub(crate) const RUN_FIXTURE: &str = r#"{"id":"run_1","object":"eval.run","created_at":1700000000,"status":"queued","data_source":{},"eval_id":"eval_1"}"#;
pub(crate) const CANCELLED_FIXTURE: &str =
    r#"{"id":"run_1","object":"eval.run","status":"cancelled"}"#;
pub(crate) const DELETED_FIXTURE: &str =
    r#"{"run_id":"run_1","object":"eval.run.deleted","deleted":true}"#;

// One call the fake handler was asked to make, so a case can check the action, URL, headers, timeout and flag
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    pub(crate) action: &'static str,
    pub(crate) is_async: bool,
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) timeout: Option<Duration>,
}

// B9371's handlers replaced by a recorder that answers the fixtures, or the one provider error it was built with
#[derive(Default)]
pub(crate) struct Fake {
    calls: Mutex<Vec<Seen>>,
    failure: Option<ProviderError>,
}

impl Fake {
    pub(crate) fn failing(error: ProviderError) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(error),
        }
    }

    pub(crate) fn calls(&self) -> Vec<Seen> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(
        &self,
        action: &'static str,
        request: &RunHandlerRequest<'_>,
    ) -> Option<EvalHttpError> {
        let seen = Seen {
            action,
            is_async: request.is_async,
            url: request.url.to_owned(),
            headers: request.headers.clone(),
            timeout: request.timeout,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        self.failure.clone().map(|error| {
            EvalHttpError::Api(EvalsError::Provider {
                provider: Box::new(error),
            })
        })
    }
}

impl EvalRunHandlers for Fake {
    #[rustfmt::skip]
    fn get_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, _logging: &'call mut LoggingState) -> impl Future<Output = Result<Run, EvalHttpError>> + Send + 'call {
        let Some(error) = self.record("get", request) else { return ready(Ok(serde_json::from_str::<Run>(RUN_FIXTURE).expect("the run fixture is valid"))) };
        ready(Err(error))
    }

    #[rustfmt::skip]
    fn cancel_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, _logging: &'call mut LoggingState) -> impl Future<Output = Result<CancelRunResponse, EvalHttpError>> + Send + 'call {
        let Some(error) = self.record("cancel", request) else { return ready(Ok(serde_json::from_str::<CancelRunResponse>(CANCELLED_FIXTURE).expect("the cancel fixture is valid"))) };
        ready(Err(error))
    }

    #[rustfmt::skip]
    fn delete_run<'call>(&'call self, request: &'call RunHandlerRequest<'_>, _logging: &'call mut LoggingState) -> impl Future<Output = Result<RunDeleteResponse, EvalHttpError>> + Send + 'call {
        let Some(error) = self.record("delete", request) else { return ready(Ok(serde_json::from_str::<RunDeleteResponse>(DELETED_FIXTURE).expect("the delete fixture is valid"))) };
        ready(Err(error))
    }
}

// The real OpenAI evals config behind a recorder: the params validate_environment saw, the transforms it ran,
// and a marker body for cancel and delete that the transport must never send
pub(crate) struct RecordingConfig {
    inner: OpenAiEvalsConfig,
    params: Mutex<Option<Object>>,
    transforms: Mutex<Vec<&'static str>>,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            inner: OpenAiEvalsConfig {
                api_key: Some(SecretString::from("sk-evals")),
                openai_key: None,
                env_api_key: None,
            },
            params: Mutex::new(None),
            transforms: Mutex::new(Vec::new()),
        }
    }
}

impl RecordingConfig {
    // the same config without any key, so validate_environment takes its missing-key branch
    pub(crate) fn without_key() -> Self {
        Self {
            inner: OpenAiEvalsConfig::default(),
            params: Mutex::new(None),
            transforms: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn params(&self) -> Option<Object> {
        self.params
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn transforms(&self) -> Vec<&'static str> {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, transform: &'static str) {
        self.transforms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(transform);
    }
}

pub(crate) fn marker_body() -> Object {
    serde_json::from_str(r#"{"marker":"a body the transport never sends"}"#)
        .expect("the marker body is valid JSON")
}

macro_rules! delegated_requests {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $out:ty;)+) => {
        $(
            fn $name(
                &self,
                $($arg: $ty,)*
                context: &EvalsRequestContext<'_>,
            ) -> Result<$out, EvalsError> {
                self.inner.$name($($arg,)* context)
            }
        )+
    };
}

macro_rules! delegated_replies {
    ($($name:ident -> $out:ty;)+) => {
        $(
            fn $name(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<$out, EvalsError> {
                self.inner.$name(raw_response)
            }
        )+
    };
}

impl BaseEvalsAPIConfig for RecordingConfig {
    type Error = EvalsError;

    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, EvalsError> {
        *self.params.lock().unwrap_or_else(PoisonError::into_inner) = litellm_params.cloned();
        self.inner.validate_environment(headers, litellm_params)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, EvalsError> {
        self.inner.get_complete_url(api_base, endpoint, eval_id)
    }

    delegated_requests! {
        transform_create_eval_request(create_request: &CreateEvalRequest) -> Object;
        transform_list_evals_request(list_params: &ListEvalsParams) -> (String, Object);
        transform_get_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_update_eval_request(eval_id: &str, update_request: &UpdateEvalRequest, api_base: &str) -> (String, Headers, Object);
        transform_delete_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_eval_request(eval_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_create_run_request(eval_id: &str, create_request: &CreateRunRequest) -> (String, Object);
        transform_list_runs_request(eval_id: &str, list_params: &ListRunsParams) -> (String, Object);
        transform_get_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers);
    }

    #[rustfmt::skip]
    fn transform_cancel_run_request(&self, eval_id: &str, run_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), EvalsError> {
        self.record("cancel_run_request");
        let (url, headers, _built) = self.inner.transform_cancel_run_request(eval_id, run_id, api_base, context)?;
        Ok((url, headers, marker_body()))
    }

    #[rustfmt::skip]
    fn transform_delete_run_request(&self, eval_id: &str, run_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), EvalsError> {
        self.record("delete_run_request");
        let (url, headers, _built) = self.inner.transform_delete_run_request(eval_id, run_id, api_base, context)?;
        Ok((url, headers, marker_body()))
    }

    delegated_replies! {
        transform_create_eval_response -> Eval;
        transform_list_evals_response -> ListEvalsResponse;
        transform_get_eval_response -> Eval;
        transform_update_eval_response -> Eval;
        transform_delete_eval_response -> DeleteEvalResponse;
        transform_cancel_eval_response -> CancelEvalResponse;
        transform_create_run_response -> Run;
        transform_list_runs_response -> ListRunsResponse;
        transform_get_run_response -> Run;
        transform_cancel_run_response -> CancelRunResponse;
        transform_delete_run_response -> RunDeleteResponse;
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> EvalsError {
        self.inner
            .get_error_class(error_message, status_code, headers)
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The pre-call environment a handler reads: one redactor and one sink, with nothing logged
pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
}

impl Default for LogEnv {
    fn default() -> Self {
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
        }
    }
}

impl LogEnv {
    pub(crate) fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
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
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }
}

pub(crate) fn state(call_type: &str) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(&format!(
        r#"{{"model":"eval","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
    ))
    .expect("the logging fixture is valid");
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging state builds")
}

pub(crate) fn provider_error(status: u16, message: &str) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: status,
        message: message.to_owned(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}

pub(crate) fn keywords(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture is valid")
}

pub(crate) fn body_json(request: &Request) -> Json {
    serde_json::from_str(&request.body).expect("the request body is JSON")
}

pub(crate) fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) fn json_server(status: &str, body: &str, pause: Duration) -> Serving {
    serve(
        reply(
            status,
            &[("content-type", "application/json"), ("x-upstream", "yes")],
            body,
        ),
        pause,
    )
    .expect("the fixture serves")
}

pub(crate) fn url(serving: &Serving, path: &str) -> String {
    format!("http://{}{path}", serving.address)
}
