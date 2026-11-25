#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_http::interactions::base::{ClientError, Params};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision as PrivacyDecision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use secrecy::SecretString;

use crate::api::decisions_send::clients::{
    Answer, DecisionsAsyncTransport, DecisionsRequest, DecisionsSyncTransport,
};
use crate::api::decisions_send::format::DecisionsOutput;
use crate::decisions::{
    DecisionsArgs, DecisionsDeps, DecisionsEntryError, DecisionsPrepare, adecisions, decisions,
};
use crate::systemone::{
    DecisionsResponse, SystemOneArgs, SystemOneEntryError, asystemone, systemone,
};

type DecisionResult = Result<DecisionsOutput, DecisionsEntryError>;
type SystemOneResult = Result<DecisionsResponse, SystemOneEntryError>;

pub(crate) const API_BASE: &str = "http://127.0.0.1:9";
pub(crate) const API_KEY: &str = "test-key";
pub(crate) const SYSTEMONE_ANSWER: &str = r#"{"model":"model-1","answers":{"first":{"type":"noul","noul":0.9}},"usage":{"input_tokens":3,"output_tokens":5}}"#;
pub(crate) const OPENAI_ANSWER: &str = r#"{"model":"model-1","answers":[{"type":"predicate","name":"first","probability":0.9}],"usage":{"input_tokens":3,"output_tokens":5,"total_tokens":8}}"#;
pub(crate) const REJECTION: &str =
    r#"{"error":{"message":"invalid api key","type":"authentication_error"}}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Sync,
    Async,
}

#[derive(Debug)]
pub(crate) struct SentCall {
    pub(crate) mode: Mode,
    pub(crate) request: DecisionsRequest,
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

pub(crate) struct FakeClients {
    calls: Arc<Mutex<Vec<SentCall>>>,
    answer: Arc<Mutex<Reply>>,
}

impl FakeClients {
    fn new(answer: &str) -> Self {
        Self {
            calls: Arc::default(),
            answer: Arc::new(Mutex::new(Reply {
                status: 200,
                body: answer.as_bytes().to_vec(),
            })),
        }
    }

    pub(crate) fn answer(&self, status: u16, body: &str) {
        let mut reply = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        reply.status = status;
        reply.body = body.as_bytes().to_vec();
    }

    pub(crate) fn call(&self) -> MutexGuard<'_, Vec<SentCall>> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn delivered(&self, mode: Mode, request: DecisionsRequest) -> UpstreamResponse {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentCall { mode, request });
        let reply = self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        UpstreamResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.clone(),
        }
    }
}

impl DecisionsSyncTransport for FakeClients {
    fn send_sync(
        &mut self,
        _params: &Params,
        request: DecisionsRequest,
    ) -> Result<Answer, ClientError> {
        Ok(Ok(self.delivered(Mode::Sync, request)))
    }
}

impl DecisionsAsyncTransport for FakeClients {
    fn send_async(
        &mut self,
        _params: &Params,
        request: DecisionsRequest,
    ) -> Result<impl Future<Output = Answer> + Send, ClientError> {
        Ok(ready(Ok(self.delivered(Mode::Async, request))))
    }
}

#[derive(Default)]
pub(crate) struct Environment {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    variables: BTreeMap<String, String>,
}

impl Environment {
    pub(crate) fn context(&self) -> ResolutionContext<'_> {
        let registry = JSONProviderRegistry::default();
        let policies = ProviderPolicies::default();
        let globals = GlobalCredentials::default();
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.variables,
            registry,
            policies,
            globals,
        }
    }
}

struct NullSink;

impl LogSink for NullSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct LogEnv {
    redactor: Redactor,
    sink: NullSink,
    now: DateTime<FixedOffset>,
}

impl LogEnv {
    fn new() -> Self {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        Self {
            redactor,
            sink: NullSink,
            now,
        }
    }

    pub(crate) fn env(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: PrivacyDecision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"decision","call_type":"decisions","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1"}"#,
    )
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

pub(crate) fn prepare<'deps>(
    resolution: &'deps ResolutionContext<'deps>,
    logging: &'deps Mutex<LoggingState>,
) -> DecisionsPrepare<'deps> {
    DecisionsPrepare {
        resolution,
        globals: LiteLlmGlobals::default(),
        azure_ai: AzureAiConfig {
            complete_url: Box::new(|base: &str, _model: &str| base.to_owned()),
        },
        logging: Some(logging),
        global_drop_params: false,
    }
}

pub(crate) fn parts<'fixture>(
    environment: &'fixture Environment,
    log: &'fixture LogEnv,
) -> (ResolutionContext<'fixture>, PrePostEnv<'fixture>) {
    (environment.context(), log.env())
}

const fn entry_deps<'deps>(
    clients: &'deps mut FakeClients,
    env: &'deps PrePostEnv<'deps>,
    caller: DecisionsPrepare<'deps>,
) -> DecisionsDeps<'deps, FakeClients> {
    DecisionsDeps {
        clients,
        env,
        prepare: caller,
    }
}

pub(crate) struct Fixture {
    pub(crate) clients: FakeClients,
    pub(crate) environment: Environment,
    pub(crate) log: LogEnv,
    pub(crate) logging: Mutex<LoggingState>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let clients = FakeClients::new(SYSTEMONE_ANSWER);
        let environment = Environment::default();
        let log = LogEnv::new();
        let logging = Mutex::new(logging_state());
        Self {
            clients,
            environment,
            log,
            logging,
        }
    }

    pub(crate) async fn adecisions(&mut self, args: DecisionsArgs) -> DecisionResult {
        let (resolution, env) = parts(&self.environment, &self.log);
        let caller = prepare(&resolution, &self.logging);
        let deps = entry_deps(&mut self.clients, &env, caller);
        adecisions(deps, args).await
    }

    pub(crate) fn decisions(&mut self, args: DecisionsArgs) -> DecisionResult {
        let (resolution, env) = parts(&self.environment, &self.log);
        let caller = prepare(&resolution, &self.logging);
        let deps = entry_deps(&mut self.clients, &env, caller);
        decisions(deps, args)
    }

    pub(crate) async fn asystemone(&mut self, args: SystemOneArgs) -> SystemOneResult {
        let (resolution, env) = parts(&self.environment, &self.log);
        let caller = prepare(&resolution, &self.logging);
        let deps = entry_deps(&mut self.clients, &env, caller);
        asystemone(deps, args).await
    }

    pub(crate) fn systemone(&mut self, args: SystemOneArgs) -> SystemOneResult {
        let (resolution, env) = parts(&self.environment, &self.log);
        let caller = prepare(&resolution, &self.logging);
        let deps = entry_deps(&mut self.clients, &env, caller);
        systemone(deps, args)
    }
}

pub(crate) fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

pub(crate) fn json(value: &str) -> Json {
    serde_json::from_str(value).expect("the test JSON parses")
}

pub(crate) fn openai_questions() -> Json {
    json(r#"[{"type":"predicate","name":"first","instructions":"Is it true?"}]"#)
}

pub(crate) fn systemone_questions() -> Json {
    json(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#)
}

pub(crate) fn decisions_args(provider: &str) -> DecisionsArgs {
    DecisionsArgs {
        model: String::from("model-1"),
        input: Some(text("state")),
        questions: Some(openai_questions()),
        safety_identifier: None,
        api_key: Some(SecretString::from(API_KEY.to_owned())),
        api_base: Some(String::from(API_BASE)),
        timeout: Some(Duration::from_secs(5)),
        custom_llm_provider: Some(provider.to_owned()),
        extra_headers: None,
        kwargs: Extra::new(),
    }
}

pub(crate) fn systemone_args(provider: &str) -> SystemOneArgs {
    SystemOneArgs::from(decisions_args(provider))
}

impl From<DecisionsArgs> for SystemOneArgs {
    fn from(args: DecisionsArgs) -> Self {
        Self {
            model: args.model,
            state: args.input,
            questions: Some(systemone_questions()),
            safety_identifier: args.safety_identifier,
            api_key: args.api_key,
            api_base: args.api_base,
            timeout: args.timeout,
            custom_llm_provider: args.custom_llm_provider,
            extra_headers: args.extra_headers,
            kwargs: args.kwargs,
        }
    }
}

pub(crate) fn body(request: &DecisionsRequest) -> Json {
    serde_json::from_slice(&request.body).expect("the sent body is JSON")
}
