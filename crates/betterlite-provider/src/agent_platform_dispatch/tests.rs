#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;

use super::a2a::{API_BASE_ENV, complete_a2a};
use super::call::CompletionCall;
use super::context::{DispatchContext, DispatchGlobals};
use super::error::DispatchError;
use super::handler::{ChatCompletionHandler, HandlerError};
use super::langflow::complete_langflow;
use super::langgraph::complete_langgraph;
use super::table::dispatch;
use crate::a2a_chat::auth::EntraResolver;
use crate::a2a_chat::error::{A2aError, A2aFailure};
use crate::a2a_chat::registry::{A2aAgentLookup, RegisteredA2aAgent};
use crate::langflow_chat::error::LangFlowError;
use crate::langgraph_chat::error::{LangGraphError, LangGraphFailure};

const MISSING_BASE: &str = "api_base is required for A2A provider. Either provide api_base parameter, set A2A_API_BASE environment variable, or register the agent in the proxy with model='a2a/<agent-name>'.";

struct Offline;

impl OidcHttp for Offline {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the dispatch fixture holds no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for Offline {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the dispatch fixture holds no Azure token source"),
        })
    }
}

impl OidcClock for Offline {
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
        oidc_http: Arc::new(Offline),
        azure_tokens: Arc::new(Offline),
        clock: Arc::new(Offline),
        oidc_cache: OidcTokenCache::default(),
    }
}

struct NoEntra;

impl EntraResolver for NoEntra {
    fn token(&self, _params: &Object) -> Result<SecretString, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch fixture resolves no Azure AD token"),
        })
    }

    fn auth_header(&self, _params: &Object) -> Result<BTreeMap<String, String>, CredentialsError> {
        Err(CredentialsError::ProviderFailed {
            message: String::from("the dispatch fixture resolves no hop header"),
        })
    }
}

struct Registry {
    agents: BTreeMap<String, RegisteredA2aAgent>,
}

impl Registry {
    fn new(agents: Vec<(&str, RegisteredA2aAgent)>) -> Self {
        Self {
            agents: agents
                .into_iter()
                .map(|(name, agent)| (name.to_owned(), agent))
                .collect(),
        }
    }
}

impl A2aAgentLookup for Registry {
    fn agent_by_name(&self, name: &str) -> Option<RegisteredA2aAgent> {
        self.agents.get(name).cloned()
    }
}

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    reply: String,
    failure: Option<u16>,
}

impl Recorder {
    fn answer(reply: &str) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: reply.to_owned(),
            failure: None,
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            reply: String::new(),
            failure: Some(status_code),
        }
    }

    fn calls(&self) -> Vec<CompletionCall> {
        self.calls.lock().expect("the fixture lock is held").clone()
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<String, HandlerError> {
        self.calls
            .lock()
            .expect("the fixture lock is held")
            .push(call.clone());
        match self.failure {
            Some(status_code) => Err(HandlerError::Upstream { status_code }),
            None => Ok(self.reply.clone()),
        }
    }
}

struct Harness {
    provider: &'static str,
    model: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
    optional_params: Object,
    provider_config: Option<ChatConfigTag>,
    stream: Option<bool>,
    globals_base: Option<String>,
    globals_key: Option<SecretString>,
    globals_headers: Option<BTreeMap<String, String>>,
    environment: BTreeMap<String, String>,
    registry: Option<Box<dyn A2aAgentLookup>>,
    litellm_params: Object,
    messages: Vec<Message>,
    timeout: Option<Duration>,
    secrets: SecretLookup,
    entra: NoEntra,
}

impl Harness {
    fn new(provider: &'static str, model: &str) -> Self {
        Self {
            provider,
            model: model.to_owned(),
            api_base: None,
            api_key: None,
            headers: None,
            optional_params: Object::new(),
            provider_config: None,
            stream: None,
            globals_base: None,
            globals_key: None,
            globals_headers: None,
            environment: BTreeMap::new(),
            registry: None,
            litellm_params: Object::new(),
            messages: vec![message("hi")],
            timeout: Some(Duration::from_secs(600)),
            secrets: secrets(),
            entra: NoEntra,
        }
    }

    fn context(&self) -> DispatchContext<'_> {
        DispatchContext {
            acompletion: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: self.provider,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: self.timeout,
            globals: DispatchGlobals {
                api_base: self.globals_base.as_deref(),
                api_key: self.globals_key.as_ref(),
                headers: self.globals_headers.as_ref(),
            },
            secrets: &self.secrets,
            environment: &self.environment,
            registry: self.registry.as_deref(),
            entra: &self.entra,
        }
    }
}

fn message(text: &str) -> Message {
    serde_json::from_value(json!({"role": "user", "content": text}))
        .expect("the fixture message is a chat message")
}

fn key(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(wire: &str) -> Object {
    serde_json::from_str(wire).expect("the fixture object is JSON")
}

fn agent(card: &str, params: &str) -> RegisteredA2aAgent {
    RegisteredA2aAgent {
        agent_card_params: object(card),
        litellm_params: object(params),
    }
}

fn exposed(key: Option<&SecretString>) -> Option<String> {
    key.map(|stored| stored.expose_secret().to_owned())
}

#[test]
fn a2a_takes_the_registered_agent_base_key_and_headers() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.provider_config = Some(ChatConfigTag::A2a);
    harness.stream = Some(true);
    // the A2A param mapping seeds the stream flag into optional_params, and the card answers without it
    harness.optional_params = object(r#"{"stream":true}"#);
    harness.registry = Some(Box::new(Registry::new(vec![(
        "my-agent",
        agent(
            r#"{"url":"http://card.test/agent","capabilities":{"streaming":false}}"#,
            r#"{"api_key":"card-key","headers":{"x-card":"yes"},"timeout":12}"#,
        ),
    )])));
    let recorder = Recorder::answer("hi");
    let reply = complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert_eq!(reply, "hi");
    let sent = recorder.calls();
    assert_eq!(sent.len(), 1);
    let call = &sent[0];
    assert_eq!(call.model, "my-agent");
    assert_eq!(call.api_base, "http://card.test/agent");
    assert_eq!(
        exposed(call.api_key.as_ref()),
        Some(String::from("card-key"))
    );
    assert_eq!(call.headers, Some(headers(&[("x-card", "yes")])));
    assert_eq!(call.stream, Some(true));
    assert_eq!(call.custom_llm_provider, "a2a");
    assert_eq!(call.provider_config, Some(ChatConfigTag::A2a));
    assert_eq!(
        call.optional_params,
        object(r#"{"stream":false,"timeout":12,"fake_stream":true}"#)
    );
    assert_eq!(call.messages, vec![message("hi")]);
}

#[test]
fn a2a_keeps_the_callers_optional_params_unmutated() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.registry = Some(Box::new(Registry::new(vec![(
        "my-agent",
        agent("{}", r#"{"timeout":12}"#),
    )])));
    harness.api_base = Some(String::from("http://explicit"));
    let recorder = Recorder::answer("hi");
    complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert!(harness.optional_params.is_empty());
    assert_eq!(
        recorder.calls()[0].optional_params,
        object(r#"{"timeout":12}"#)
    );
}

#[rstest]
#[case::explicit_over_global(
    Some("http://explicit.test"),
    Some("http://global.test"),
    "http://explicit.test"
)]
#[case::global_when_absent(None, Some("http://global.test"), "http://global.test")]
#[case::global_when_argument_is_empty(Some(""), Some("http://global.test"), "http://global.test")]
fn a2a_resolves_the_base_from_the_argument_then_the_globals(
    #[case] resolved: Option<&str>,
    #[case] global: Option<&str>,
    #[case] expected: &str,
) {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.api_base = resolved.map(str::to_owned);
    harness.globals_base = global.map(str::to_owned);
    let recorder = Recorder::answer("hi");
    complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert_eq!(recorder.calls()[0].api_base, expected);
}

#[test]
fn a2a_ignores_an_empty_argument_base_and_reads_the_environment() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.api_base = Some(String::new());
    harness
        .environment
        .insert(String::from(API_BASE_ENV), String::from("http://env.test"));
    let recorder = Recorder::answer("hi");
    complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert_eq!(recorder.calls()[0].api_base, "http://env.test");
}

#[test]
fn a2a_without_any_base_raises_the_litellm_text() {
    let harness = Harness::new("a2a", "my-agent");
    let recorder = Recorder::answer("hi");
    let error = complete_a2a(&harness.context(), &recorder).expect_err("the base is missing");
    assert_eq!(error.to_string(), MISSING_BASE);
    assert!(recorder.calls().is_empty());
}

#[test]
fn a2a_treats_an_empty_global_base_as_unset() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.globals_base = Some(String::new());
    harness
        .environment
        .insert(String::from(API_BASE_ENV), String::from("http://env.test"));
    let recorder = Recorder::answer("hi");
    complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert_eq!(recorder.calls()[0].api_base, "http://env.test");
}

#[test]
fn a2a_treats_an_empty_environment_base_as_unset() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness
        .environment
        .insert(String::from(API_BASE_ENV), String::new());
    let recorder = Recorder::answer("hi");
    let error = complete_a2a(&harness.context(), &recorder).expect_err("the base is missing");
    assert!(matches!(error, DispatchError::MissingA2aApiBase));
}

#[rstest]
#[case::absent(None, "x-global")]
#[case::empty(Some(&[]), "x-global")]
#[case::given(Some(&[("x-given", "1")]), "x-given")]
fn a2a_headers_take_the_global_only_when_absent_or_empty(
    #[case] given: Option<&[(&str, &str)]>,
    #[case] expected_name: &str,
) {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.api_base = Some(String::from("http://explicit.test"));
    harness.headers = given.map(headers);
    harness.globals_headers = Some(headers(&[("x-global", "1")]));
    let recorder = Recorder::answer("hi");
    complete_a2a(&harness.context(), &recorder).expect("the call resolves");
    assert_eq!(
        recorder.calls()[0].headers,
        Some(headers(&[(expected_name, "1")]))
    );
}

#[test]
fn a2a_passes_a_handler_error_through_unchanged() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.api_base = Some(String::from("http://explicit.test"));
    let recorder = Recorder::failing(502);
    let error = complete_a2a(&harness.context(), &recorder).expect_err("the handler fails");
    match error {
        DispatchError::Handler(HandlerError::Upstream { status_code }) => {
            assert_eq!(status_code, 502);
        }
        other => panic!("the handler error is hidden: {other}"),
    }
    assert_eq!(recorder.calls().len(), 1);
}

#[test]
fn a2a_surfaces_a_registry_failure() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.registry = Some(Box::new(Registry::new(vec![(
        "my-agent",
        agent("{}", r#"{"headers":{"X-Trace":7}}"#),
    )])));
    let recorder = Recorder::answer("hi");
    let error = complete_a2a(&harness.context(), &recorder).expect_err("the header is not text");
    assert!(matches!(
        error,
        DispatchError::A2a(A2aFailure::HeaderType { .. })
    ));
    assert!(recorder.calls().is_empty());
}

#[test]
fn langgraph_passes_the_provider_info_pair() {
    let mut harness = Harness::new("langgraph", "agent");
    harness.api_base = Some(String::from("http://lg.test"));
    harness.api_key = Some(key("lg-key"));
    harness.headers = Some(headers(&[("x-lg", "1")]));
    harness.provider_config = Some(ChatConfigTag::LangGraph);
    let recorder = Recorder::answer("hi");
    complete_langgraph(&harness.context(), &recorder).expect("the call resolves");
    let sent = recorder.calls();
    let call = &sent[0];
    assert_eq!(call.model, "agent");
    assert_eq!(call.api_base, "http://lg.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some(String::from("lg-key")));
    assert_eq!(call.headers, Some(headers(&[("x-lg", "1")])));
    assert_eq!(call.custom_llm_provider, "langgraph");
    assert!(call.provider_config.is_none());
    assert_eq!(call.optional_params, Object::new());
}

#[test]
fn langgraph_reads_the_environment_pair() {
    let mut harness = Harness::new("langgraph", "agent");
    harness.environment.extend([
        (
            String::from("LANGGRAPH_API_BASE"),
            String::from("http://env-lg.test"),
        ),
        (
            String::from("LANGGRAPH_API_KEY"),
            String::from("env-lg-key"),
        ),
    ]);
    let recorder = Recorder::answer("hi");
    complete_langgraph(&harness.context(), &recorder).expect("the call resolves");
    let call = &recorder.calls()[0];
    assert_eq!(call.api_base, "http://env-lg.test");
    assert_eq!(
        exposed(call.api_key.as_ref()),
        Some(String::from("env-lg-key"))
    );
}

#[test]
fn langgraph_defaults_the_base_to_the_local_runtime() {
    let harness = Harness::new("langgraph", "agent");
    let recorder = Recorder::answer("hi");
    complete_langgraph(&harness.context(), &recorder).expect("the call resolves");
    let call = &recorder.calls()[0];
    assert_eq!(call.api_base, "http://localhost:2024");
    assert!(call.api_key.is_none());
}

#[test]
fn langgraph_prefers_a_global_base_and_key_before_the_environment() {
    let mut harness = Harness::new("langgraph", "agent");
    harness.globals_base = Some(String::from("http://global-lg.test"));
    harness.globals_key = Some(key("global-lg-key"));
    harness.environment.extend([
        (
            String::from("LANGGRAPH_API_BASE"),
            String::from("http://env-lg.test"),
        ),
        (
            String::from("LANGGRAPH_API_KEY"),
            String::from("env-lg-key"),
        ),
    ]);
    let recorder = Recorder::answer("hi");
    complete_langgraph(&harness.context(), &recorder).expect("the call resolves");
    let call = &recorder.calls()[0];
    assert_eq!(call.api_base, "http://global-lg.test");
    assert_eq!(
        exposed(call.api_key.as_ref()),
        Some(String::from("global-lg-key"))
    );
}

#[test]
fn langgraph_passes_a_handler_error_through_unchanged() {
    let harness = Harness::new("langgraph", "agent");
    let recorder = Recorder::failing(503);
    let error = complete_langgraph(&harness.context(), &recorder).expect_err("the handler fails");
    match error {
        DispatchError::Handler(HandlerError::Upstream { status_code }) => {
            assert_eq!(status_code, 503);
        }
        other => panic!("the handler error is hidden: {other}"),
    }
}

#[test]
fn langflow_passes_the_provider_info_pair() {
    let mut harness = Harness::new("langflow", "flow-1");
    harness.api_base = Some(String::from("http://lf.test"));
    harness.api_key = Some(key("lf-key"));
    harness.provider_config = Some(ChatConfigTag::LangFlow);
    let recorder = Recorder::answer("hello");
    complete_langflow(&harness.context(), &recorder).expect("the call resolves");
    let sent = recorder.calls();
    let call = &sent[0];
    assert_eq!(call.model, "flow-1");
    assert_eq!(call.api_base, "http://lf.test");
    assert_eq!(exposed(call.api_key.as_ref()), Some(String::from("lf-key")));
    assert_eq!(call.custom_llm_provider, "langflow");
    assert!(call.provider_config.is_none());
}

#[test]
fn langflow_reads_the_environment_pair_and_defaults_the_base() {
    let mut from_environment = Harness::new("langflow", "flow-1");
    from_environment.environment.extend([
        (
            String::from("LANGFLOW_API_BASE"),
            String::from("http://env-lf.test"),
        ),
        (String::from("LANGFLOW_API_KEY"), String::from("env-lf-key")),
    ]);
    let recorder = Recorder::answer("hello");
    complete_langflow(&from_environment.context(), &recorder).expect("the call resolves");
    let call = &recorder.calls()[0];
    assert_eq!(call.api_base, "http://env-lf.test");
    assert_eq!(
        exposed(call.api_key.as_ref()),
        Some(String::from("env-lf-key"))
    );

    let bare = Harness::new("langflow", "flow-1");
    let other = Recorder::answer("hello");
    complete_langflow(&bare.context(), &other).expect("the call resolves");
    assert_eq!(other.calls()[0].api_base, "http://localhost:7860");
}

#[test]
fn langflow_passes_a_handler_error_through_unchanged() {
    let harness = Harness::new("langflow", "flow-1");
    let recorder = Recorder::failing(500);
    let error = complete_langflow(&harness.context(), &recorder).expect_err("the handler fails");
    match error {
        DispatchError::Handler(HandlerError::Upstream { status_code }) => {
            assert_eq!(status_code, 500);
        }
        other => panic!("the handler error is hidden: {other}"),
    }
}

#[test]
fn handler_error_reports_the_provider_status() {
    let upstream = HandlerError::Upstream { status_code: 502 };
    assert_eq!(upstream.status_code(), Some(502));
    let a2a_reply = HandlerError::A2aReply(A2aError {
        status_code: 503,
        message: String::from("the agent answered an error"),
        headers: BTreeMap::new(),
    });
    assert_eq!(a2a_reply.status_code(), Some(503));
    let langgraph_reply = HandlerError::LangGraphReply(LangGraphError {
        status_code: 504,
        message: String::from("the runtime answered an error"),
    });
    assert_eq!(langgraph_reply.status_code(), Some(504));
    let langflow = HandlerError::LangFlow(LangFlowError::Api {
        status_code: 505,
        message: String::from("the flow answered an error"),
    });
    assert_eq!(langflow.status_code(), Some(505));
    let a2a = HandlerError::A2a(A2aFailure::MessagesRequired);
    assert_eq!(a2a.status_code(), None);
    let langgraph = HandlerError::LangGraph(LangGraphFailure::MissingApiBase);
    assert_eq!(langgraph.status_code(), None);
    assert_eq!(HandlerError::EmptyReply.status_code(), None);
}

#[test]
fn dispatch_reaches_the_a2a_arm() {
    let mut harness = Harness::new("a2a", "my-agent");
    harness.api_base = Some(String::from("http://explicit.test"));
    harness.provider_config = Some(ChatConfigTag::A2a);
    let recorder = Recorder::answer("hi");
    let found = dispatch(ChatConfigTag::A2a, &harness.context(), &recorder);
    assert!(found.expect("the tag has an arm").is_ok());
    assert_eq!(recorder.calls()[0].custom_llm_provider, "a2a");
    assert_eq!(
        recorder.calls()[0].provider_config,
        Some(ChatConfigTag::A2a)
    );
}

#[test]
fn dispatch_reaches_the_langgraph_arm() {
    let harness = Harness::new("langgraph", "agent");
    let recorder = Recorder::answer("hi");
    let found = dispatch(ChatConfigTag::LangGraph, &harness.context(), &recorder);
    assert!(found.expect("the tag has an arm").is_ok());
    assert_eq!(recorder.calls()[0].custom_llm_provider, "langgraph");
    assert!(recorder.calls()[0].provider_config.is_none());
}

#[test]
fn dispatch_reaches_the_langflow_arm() {
    let harness = Harness::new("langflow", "flow-1");
    let recorder = Recorder::answer("hi");
    let found = dispatch(ChatConfigTag::LangFlow, &harness.context(), &recorder);
    assert!(found.expect("the tag has an arm").is_ok());
    assert_eq!(recorder.calls()[0].custom_llm_provider, "langflow");
    assert!(recorder.calls()[0].provider_config.is_none());
}

#[rstest]
#[case::ragflow(ChatConfigTag::RagFlow)]
#[case::anthropic(ChatConfigTag::Anthropic)]
fn dispatch_leaves_every_other_provider_alone(#[case] tag: ChatConfigTag) {
    let harness = Harness::new("other", "model");
    let recorder = Recorder::answer("hi");
    assert!(dispatch(tag, &harness.context(), &recorder).is_none());
    assert!(recorder.calls().is_empty());
}
