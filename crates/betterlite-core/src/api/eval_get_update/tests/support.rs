use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::core_settings::CoreSettings;
use betterlite_context::resource_config_selection::skills_evals::EvalsConfigTag;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
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
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::task::yield_now;

use crate::api::eval_create_list::call::EvalsCallParams;
use crate::api::eval_create_list::config::EvalsConfigLookup;
use crate::api::eval_get_update::call::{GetEvalCall, UpdateEvalCall};
use crate::api::eval_get_update::handler::{
    EvalsGetUpdateHandler, GetEvalHttpCall, UpdateEvalHttpCall,
};
use crate::api::eval_get_update::service::EvalsGetUpdateService;
use crate::api::eval_http::error::EvalHttpError;
use crate::api::eval_http::exchange::EvalConfig;

// what validate_environment was handed: the caller's headers and the kwargs object GenericLiteLLMParams would parse
#[derive(Clone, Debug, Default)]
pub(crate) struct Validated {
    pub(crate) headers_in: Headers,
    pub(crate) litellm_params: Option<Object>,
}

// the arguments the get transform was handed
#[derive(Clone, Debug)]
pub(crate) struct SeenGet {
    pub(crate) eval_id: String,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) litellm_params: Object,
}

// the typed request and the context the update transform was handed
#[derive(Clone, Debug)]
pub(crate) struct SeenUpdate {
    pub(crate) eval_id: String,
    pub(crate) request: UpdateEvalRequest,
    pub(crate) api_base: String,
    pub(crate) headers: Headers,
    pub(crate) litellm_params: Object,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Recorded {
    pub(crate) validated: Vec<Validated>,
    pub(crate) gets: Vec<SeenGet>,
    pub(crate) updates: Vec<SeenUpdate>,
}

// the real OpenAI config behind a recorder for the four calls the operations make
pub(crate) struct RecordingConfig {
    inner: OpenAiEvalsConfig,
    recorded: Arc<Mutex<Recorded>>,
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

macro_rules! delegated_responses {
    ($($name:ident -> $out:ty;)+) => {
        $(
            fn $name(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<$out, EvalsError> {
                self.inner.$name(raw_response)
            }
        )+
    };
}

// the get and update transforms record what they were handed, then delegate to the real OpenAI transform
macro_rules! recording_request {
    (
        $method:ident($($arg:ident: $ty:ty),+) -> $out:ty,
        $slot:ident,
        $seen:ident $(, $field:ident = $value:expr)*
    ) => {
        fn $method(
            &self,
            $($arg: $ty,)+
            context: &EvalsRequestContext<'_>,
        ) -> Result<$out, EvalsError> {
            self.recorded
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .$slot
                .push($seen {
                    $($field: $value,)*
                    headers: context.headers.clone(),
                    litellm_params: context.litellm_params.clone(),
                });
            self.inner.$method($($arg,)+ context)
        }
    };
}

impl RecordingConfig {
    pub(crate) fn new(inner: OpenAiEvalsConfig) -> Self {
        Self {
            inner,
            recorded: Arc::default(),
        }
    }

    pub(crate) fn recorded(&self) -> Recorded {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BaseEvalsAPIConfig for RecordingConfig {
    type Error = EvalsError;

    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, EvalsError> {
        let resolved = self
            .inner
            .validate_environment(headers.clone(), litellm_params);
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .validated
            .push(Validated {
                headers_in: headers,
                litellm_params: litellm_params.cloned(),
            });
        resolved
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, EvalsError> {
        self.inner.get_complete_url(api_base, endpoint, eval_id)
    }

    recording_request!(
        transform_get_eval_request(eval_id: &str, api_base: &str) -> (String, Headers),
        gets,
        SeenGet,
        eval_id = eval_id.to_owned(),
        api_base = api_base.to_owned()
    );

    recording_request!(
        transform_update_eval_request(
            eval_id: &str,
            update_request: &UpdateEvalRequest,
            api_base: &str
        ) -> (String, Headers, Object),
        updates,
        SeenUpdate,
        eval_id = eval_id.to_owned(),
        request = update_request.clone(),
        api_base = api_base.to_owned()
    );

    delegated_requests! {
        transform_create_eval_request(create_request: &CreateEvalRequest) -> Object;
        transform_list_evals_request(list_params: &ListEvalsParams) -> (String, Object);
        transform_delete_eval_request(eval_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_eval_request(eval_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_create_run_request(eval_id: &str, create_request: &CreateRunRequest) -> (String, Object);
        transform_list_runs_request(eval_id: &str, list_params: &ListRunsParams) -> (String, Object);
        transform_get_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers);
        transform_cancel_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
        transform_delete_run_request(eval_id: &str, run_id: &str, api_base: &str) -> (String, Headers, Object);
    }

    delegated_responses! {
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
}

pub(crate) struct FakeConfigs {
    pub(crate) config: Option<RecordingConfig>,
}

impl FakeConfigs {
    pub(crate) fn recording(config: RecordingConfig) -> Self {
        Self {
            config: Some(config),
        }
    }

    pub(crate) fn missing() -> Self {
        Self { config: None }
    }
}

impl EvalsConfigLookup for FakeConfigs {
    fn evals_config(&self, tag: EvalsConfigTag) -> Option<EvalConfig<'_>> {
        match tag {
            EvalsConfigTag::OpenAI => {
                let config: EvalConfig<'_> = self.config.as_ref()?;
                Some(config)
            }
        }
    }
}

// what the handler was handed and what the logging state held when the operation reached it
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GetRecord {
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) timeout: Duration,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) provider: Option<String>,
    pub(crate) litellm_params: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UpdateRecord {
    pub(crate) url: String,
    pub(crate) headers: Headers,
    pub(crate) body: Object,
    pub(crate) timeout: Duration,
    pub(crate) optional_params: IndexMap<String, Json>,
    pub(crate) provider: Option<String>,
    pub(crate) litellm_params: IndexMap<String, Json>,
}

// B9370's handler behind a seam: records every call, may suspend first, and answers from a queue
#[derive(Default)]
pub(crate) struct FakeHandler {
    gets: Mutex<Vec<GetRecord>>,
    updates: Mutex<Vec<UpdateRecord>>,
    get_answers: Mutex<VecDeque<Result<Eval, EvalHttpError>>>,
    update_answers: Mutex<VecDeque<Result<Eval, EvalHttpError>>>,
    suspends: AtomicUsize,
    completed: AtomicUsize,
}

impl FakeHandler {
    pub(crate) fn answer_get(&self, answer: Result<Eval, EvalHttpError>) {
        self.get_answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(answer);
    }

    pub(crate) fn answer_update(&self, answer: Result<Eval, EvalHttpError>) {
        self.update_answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(answer);
    }

    pub(crate) fn suspend(&self, times: usize) {
        self.suspends.store(times, Ordering::SeqCst);
    }

    pub(crate) fn gets(&self) -> Vec<GetRecord> {
        self.gets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn updates(&self) -> Vec<UpdateRecord> {
        self.updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn completed(&self) -> usize {
        self.completed.load(Ordering::SeqCst)
    }

    async fn pause(&self) {
        let mut left = self.suspends.load(Ordering::SeqCst);
        while left > 0 {
            yield_now().await;
            left = left.saturating_sub(1);
        }
    }
}

// the get and update arms record the call, pause when a test asked, and answer from the queue
macro_rules! forwarding_handler {
    (
        $method:ident($call:ident) -> $response:ty,
        $records:ident,
        $answers:ident,
        $record:ident
        $(, $payload:ident)*
    ) => {
        async fn $method(&self, call: $call<'_, '_>) -> Result<$response, EvalHttpError> {
            let $call {
                connection,
                logging,
                timeout,
                $($payload,)*
            } = call;
            self.$records
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push($record {
                    url: connection.url.to_owned(),
                    headers: connection.headers.clone(),
                    $($payload: $payload.clone(),)*
                    timeout,
                    optional_params: logging.state.optional_params.clone(),
                    provider: logging.state.custom_llm_provider.clone(),
                    litellm_params: logging.state.litellm_params.clone(),
                });
            self.pause().await;
            self.completed.fetch_add(1, Ordering::SeqCst);
            self.$answers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .expect("the handler answers a queued result")
        }
    };
}

impl EvalsGetUpdateHandler for FakeHandler {
    forwarding_handler!(get_eval(GetEvalHttpCall) -> Eval, gets, get_answers, GetRecord);

    forwarding_handler!(
        update_eval(UpdateEvalHttpCall) -> Eval,
        updates,
        update_answers,
        UpdateRecord,
        body
    );
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Self {
        Self {
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
            sink: Sink,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .expect("the clock parses"),
        }
    }

    pub(crate) fn pre_post(&self) -> PrePostEnv<'_> {
        let decision = Decision {
            content: false,
            key_info: false,
        };
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision,
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

pub(crate) struct Harness {
    pub(crate) configs: FakeConfigs,
    pub(crate) handler: FakeHandler,
    pub(crate) env: LogEnv,
    pub(crate) client: ProxiedClient,
    pub(crate) core: CoreSettings,
    pub(crate) redactor: Redactor,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self::with_credentials(OpenAiEvalsConfig {
            api_key: Some(SecretString::from("sk-test")),
            openai_key: None,
            env_api_key: None,
        })
    }

    pub(crate) fn without_credentials() -> Self {
        Self::with_credentials(OpenAiEvalsConfig::default())
    }

    pub(crate) fn without_config() -> Self {
        Self::with_configs(FakeConfigs::missing())
    }

    fn with_credentials(config: OpenAiEvalsConfig) -> Self {
        Self::with_configs(FakeConfigs::recording(RecordingConfig::new(config)))
    }

    fn with_configs(configs: FakeConfigs) -> Self {
        Self {
            configs,
            handler: FakeHandler::default(),
            env: LogEnv::new(),
            client: proxied_client(),
            core: CoreSettings {
                request_timeout_seconds: 6000.0_f64,
                request_timeout_explicit: false,
                litellm_request_timeout: None,
                model_timeout_overrides: Vec::new(),
            },
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds"),
        }
    }

    pub(crate) fn service(&self) -> EvalsGetUpdateService<'_, FakeHandler, FakeConfigs> {
        EvalsGetUpdateService {
            handler: &self.handler,
            configs: &self.configs,
            client: &self.client,
            core: &self.core,
            redactor: &self.redactor,
        }
    }

    pub(crate) fn recorded(&self) -> Recorded {
        self.configs
            .config
            .as_ref()
            .map_or_else(Recorded::default, RecordingConfig::recorded)
    }

    pub(crate) fn logging_state(&self, call_type: &str) -> LoggingState {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"eval","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let mut context = CorrelationContext::default();
        LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            &self.redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds")
    }
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(
        base_builder().http1_only(),
        &policy,
        TransportLimits::DEFAULT,
    )
    .expect("the proxied client builds")
}

// the arguments @client injects, which every operation forwards to the same EvalsCallParams
#[derive(Default)]
pub(crate) struct Calls {
    pub(crate) extra_headers: Option<Headers>,
    pub(crate) extra_query: IndexMap<String, Json>,
    pub(crate) extra_body: IndexMap<String, Json>,
    pub(crate) kwargs: IndexMap<String, Json>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) provider: Option<String>,
}

impl Calls {
    // field_reassign_with_default rejects a Default value followed by field assignment, so tests start here
    pub(crate) fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn params(&self) -> EvalsCallParams<'_> {
        EvalsCallParams {
            extra_headers: self.extra_headers.as_ref(),
            extra_query: &self.extra_query,
            timeout: self.timeout,
            custom_llm_provider: self.provider.as_deref(),
            kwargs: &self.kwargs,
        }
    }
}

pub(crate) fn get_call<'call>(calls: &'call Calls, eval_id: &'call str) -> GetEvalCall<'call> {
    GetEvalCall {
        eval_id,
        params: calls.params(),
    }
}

pub(crate) fn update_call<'call>(
    calls: &'call Calls,
    eval_id: &'call str,
    name: Option<&'call str>,
    metadata: Option<&'call IndexMap<String, Json>>,
) -> UpdateEvalCall<'call> {
    UpdateEvalCall {
        eval_id,
        name,
        metadata,
        extra_body: &calls.extra_body,
        params: calls.params(),
    }
}

pub(crate) fn eval(id: &str) -> Eval {
    let text = format!(
        r#"{{"id":"{id}","created_at":1700000000,"data_source_config":{{"type":"logs"}},"testing_criteria":[]}}"#
    );
    serde_json::from_str(&text).expect("the eval fixture is valid")
}

pub(crate) fn provider_failure(status: u16, message: &str, body: Option<Object>) -> EvalHttpError {
    let parts = ProviderErrorParts {
        status_code: status,
        message: message.to_owned(),
        headers: None,
        request: None,
        response: None,
        body,
        status_code_is_synthesized: false,
    };
    EvalHttpError::Api(EvalsError::Provider {
        provider: Box::new(ProviderError::from(parts)),
    })
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the JSON fixture is valid")
}

// the thirty keys LiteLLM never sends to OpenAI, spelled out here rather than read from the filter
pub(crate) fn internal_metadata() -> IndexMap<String, Json> {
    let keys = [
        "headers",
        "requester_metadata",
        "user_api_key_hash",
        "user_api_key_alias",
        "user_api_key_spend",
        "user_api_key_max_budget",
        "user_api_key_team_id",
        "user_api_key_user_id",
        "user_api_key_org_id",
        "user_api_key_team_alias",
        "user_api_key_end_user_id",
        "user_api_key_user_email",
        "user_api_key_request_route",
        "user_api_key_budget_reset_at",
        "user_api_key_auth_metadata",
        "user_api_key",
        "user_api_end_user_max_budget",
        "user_api_key_auth",
        "litellm_api_version",
        "global_max_parallel_requests",
        "user_api_key_team_max_budget",
        "user_api_key_team_spend",
        "user_api_key_model_max_budget",
        "user_api_key_user_spend",
        "user_api_key_user_max_budget",
        "user_api_key_metadata",
        "endpoint",
        "litellm_parent_otel_span",
        "requester_ip_address",
        "user_agent",
    ];
    keys.iter()
        .map(|name| ((*name).to_owned(), json("\"internal\"")))
        .collect()
}

pub(crate) fn map(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
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
