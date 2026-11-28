#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::call::CompletionCall;
use super::context::{DispatchContext, DispatchGlobals};
use super::dispatch;
use super::error::DispatchError;
use super::handler::{ChatCompletionHandler, HandlerError};
use crate::ragflow_chat::error::RagflowError;
use crate::ragflow_chat::tests::support::{environment, logging, messages, redactor, secrets};

struct Fixture {
    secrets: SecretLookup,
    environment: BTreeMap<String, String>,
    model: String,
    messages: Vec<Message>,
    optional_params: Object,
    litellm_params: Object,
    provider: String,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
    provider_config: Option<ChatConfigTag>,
    stream: Option<bool>,
    acompletion: bool,
}

impl Fixture {
    fn new() -> Self {
        Self {
            secrets: secrets(),
            environment: BTreeMap::new(),
            model: String::from("ragflow/chat/abc/gpt-4o"),
            messages: messages(r#"[{"role":"user","content":"hi"}]"#),
            optional_params: Object::new(),
            litellm_params: Object::new(),
            provider: String::from("ragflow"),
            api_base: None,
            api_key: None,
            headers: None,
            provider_config: None,
            stream: None,
            acompletion: false,
        }
    }

    fn context<'fixture>(
        &'fixture self,
        globals: DispatchGlobals<'fixture>,
    ) -> DispatchContext<'fixture> {
        DispatchContext {
            acompletion: self.acompletion,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: None,
            globals,
            secrets: &self.secrets,
            environment: &self.environment,
        }
    }
}

struct GlobalsFixture {
    api_base: Option<String>,
    api_key: Option<SecretString>,
    headers: Option<BTreeMap<String, String>>,
}

impl GlobalsFixture {
    const fn empty() -> Self {
        Self {
            api_base: None,
            api_key: None,
            headers: None,
        }
    }

    fn borrowed(&self) -> DispatchGlobals<'_> {
        DispatchGlobals {
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            headers: self.headers.as_ref(),
        }
    }
}

struct Recorder {
    calls: Mutex<Vec<CompletionCall>>,
    failure: Option<u16>,
}

impl Recorder {
    const fn succeeding() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: None,
        }
    }

    const fn failing(status_code: u16) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failure: Some(status_code),
        }
    }

    fn calls(&self) -> Vec<CompletionCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ChatCompletionHandler for Recorder {
    type Reply = String;

    fn complete(&self, call: &CompletionCall) -> Result<Self::Reply, HandlerError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.clone());
        self.failure.map_or_else(
            || Ok(String::from("reply")),
            |status_code| Err(HandlerError::Upstream { status_code }),
        )
    }
}

fn run_arm(
    tag: ChatConfigTag,
    fixture: &Fixture,
    globals: DispatchGlobals<'_>,
    handler: &Recorder,
) -> (
    Result<String, DispatchError>,
    Vec<CompletionCall>,
    LoggingState,
) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(tag, &fixture.context(globals), handler, &mut log)
        .expect("the ragflow tag is handled here");
    (outcome, handler.calls(), state)
}

#[test]
fn the_arm_resolves_the_environment_base_and_key_and_calls_the_handler() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Request"),
        String::from("one"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, calls, state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert_eq!(call.model, "ragflow/chat/abc/gpt-4o");
    assert_eq!(call.api_base.as_deref(), Some("http://env:9380"));
    assert_eq!(
        call.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
    assert_eq!(call.custom_llm_provider, "ragflow");
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Request")),
        Some(&String::from("one"))
    );
    assert_eq!(call.messages, fixture.messages);
    assert_eq!(call.optional_params, fixture.optional_params);
    assert_eq!(call.litellm_params, fixture.litellm_params);
    assert_eq!(call.stream, None);
    assert!(!call.acompletion);
    assert_eq!(call.provider_config, None);
    assert_eq!(state.model_call_details.get("original_response"), None);
}

#[test]
fn explicit_values_win_over_the_globals_and_the_environment() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://explicit:9380"));
    fixture.api_key = Some(SecretString::from("explicit-key"));
    fixture.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    let mut globals = GlobalsFixture::empty();
    globals.api_base = Some(String::from("http://global:9380"));
    globals.api_key = Some(SecretString::from("global-key"));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].api_base.as_deref(), Some("http://explicit:9380"));
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("explicit-key")
    );
}

#[test]
fn an_empty_explicit_base_and_key_fall_through_to_the_environment() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::new());
    fixture.api_key = Some(SecretString::from(""));
    fixture.environment = environment(&[
        ("RAGFLOW_API_BASE", "http://env:9380"),
        ("RAGFLOW_API_KEY", "env-key"),
    ]);
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].api_base.as_deref(), Some("http://env:9380"));
    assert_eq!(
        calls[0].api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-key")
    );
}

#[test]
fn the_global_headers_are_not_merged_into_the_call() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Request"),
        String::from("one"),
    )]));
    let mut globals = GlobalsFixture::empty();
    globals.headers = Some(BTreeMap::from([(
        String::from("X-Global"),
        String::from("two"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        globals.borrowed(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    let headers = calls[0]
        .headers
        .as_ref()
        .expect("the call carries the context headers");
    assert_eq!(headers.get("X-Request"), Some(&String::from("one")));
    assert_eq!(headers.get("X-Global"), None);
}

#[test]
fn an_absent_context_header_list_stays_absent() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].headers, None);
}

#[test]
fn a_bad_model_is_refused_before_the_handler_and_logs_nothing() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("ragflow/chat");
    let handler = Recorder::succeeding();
    let (outcome, calls, state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    match outcome {
        Err(DispatchError::Provider(RagflowError::ModelTooShort { .. })) => {}
        other => panic!("expected the model error, got {other:?}"),
    }
    assert!(calls.is_empty());
    assert_eq!(state.model_call_details.get("original_response"), None);
}

#[test]
fn a_handler_error_is_logged_and_returned_unchanged() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Request-Tag"),
        String::from("one"),
    )]));
    let handler = Recorder::failing(500);
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    match outcome {
        Err(DispatchError::Handler(HandlerError::Upstream { status_code })) => {
            assert_eq!(status_code, 500);
        }
        other => panic!("expected the handler failure, got {other:?}"),
    }
    let details = &state.model_call_details;
    let error_text = Json::Text(String::from("the provider answered with status 500"));
    assert_eq!(details.get("original_response"), Some(&error_text));
    assert_eq!(
        details.get("log_event_type"),
        Some(&Json::Text(String::from("post_api_call")))
    );
    let Some(Json::List(input)) = details.get("input") else {
        panic!("the post_call log holds the message list");
    };
    assert_eq!(input.len(), 1);
    let Some(Json::Object(args)) = details.get("additional_args") else {
        panic!("the post_call log holds the additional arguments");
    };
    let Some(Json::Object(headers)) = args.get("headers") else {
        panic!("the post_call log holds the headers");
    };
    assert_eq!(
        headers.get("X-Request-Tag"),
        Some(&Json::Text(String::from("one")))
    );
}

#[test]
fn a_handler_error_without_context_headers_logs_no_header_key() {
    let fixture = Fixture::new();
    let handler = Recorder::failing(502);
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert!(matches!(
        outcome,
        Err(DispatchError::Handler(HandlerError::Upstream {
            status_code: 502
        }))
    ));
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the post_call log holds the additional arguments");
    };
    assert_eq!(args.get("headers"), None);
}

#[test]
fn a_successful_call_logs_nothing() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _calls, state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(state.model_call_details.get("original_response"), None);
}

#[rstest]
#[case::openai(ChatConfigTag::OpenAiGpt)]
#[case::databricks(ChatConfigTag::Databricks)]
fn another_provider_tag_is_left_to_its_own_arm(#[case] tag: ChatConfigTag) {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(
        tag,
        &fixture.context(DispatchGlobals::default()),
        &handler,
        &mut log,
    );
    assert!(outcome.is_none());
    assert!(handler.calls().is_empty());
}

#[test]
fn the_literal_provider_and_the_selected_config_reach_the_handler() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("azure");
    fixture.provider_config = Some(ChatConfigTag::A2a);
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].custom_llm_provider, "ragflow");
    assert_eq!(calls[0].provider_config, Some(ChatConfigTag::A2a));
}

#[test]
fn the_stream_and_acompletion_flags_reach_the_handler() {
    let mut fixture = Fixture::new();
    fixture.stream = Some(true);
    fixture.acompletion = true;
    let handler = Recorder::succeeding();
    let (outcome, calls, _state) = run_arm(
        ChatConfigTag::RagFlow,
        &fixture,
        DispatchGlobals::default(),
        &handler,
    );
    assert_eq!(outcome.ok(), Some(String::from("reply")));
    assert_eq!(calls[0].stream, Some(true));
    assert!(calls[0].acompletion);
}
