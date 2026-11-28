use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as LogJson;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;
use serde_json::Number;

use crate::petals::error::{ENGINE_MISSING_MESSAGE, PetalsSetupError};
use crate::self_hosted_dispatch::call::{CompletionCall, SelfHostedConfig};
use crate::self_hosted_dispatch::chat::{complete_lemonade, dispatch};
use crate::self_hosted_dispatch::context::{DispatchContext, DispatchSettings};
use crate::self_hosted_dispatch::error::DispatchError;
use crate::self_hosted_dispatch::handler::HandlerError;
use crate::vllm::completion::ENGINE_ERROR;
use crate::vllm::error::VllmError;

use super::section::SectionFixture;
use super::support::{Fixture, Recorder, logging, redactor, secret, secret_text};

// the request context one chat arm reads, built from the fixture's values
fn chat_context(fixture: &Fixture) -> DispatchContext<'_> {
    DispatchContext {
        acompletion: fixture.acompletion,
        api_base: fixture.api_base.as_deref(),
        api_key: fixture.api_key.as_ref(),
        custom_llm_provider: &fixture.provider,
        headers: fixture.headers.as_ref(),
        litellm_params: &fixture.litellm_params,
        messages: &fixture.messages,
        model: &fixture.model,
        optional_params: &fixture.optional_params,
        stream: fixture.stream,
        timeout: fixture.timeout,
        settings: DispatchSettings::from(&fixture.settings),
    }
}

fn run(
    tag: ChatConfigTag,
    fixture: &Fixture,
    handler: &Recorder,
) -> (Option<Result<String, DispatchError>>, LoggingState) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let outcome = {
        let mut log = LogContext {
            state: &mut state,
            env: &env,
        };
        dispatch(tag, &chat_context(fixture), handler, &mut log)
    };
    (outcome, state)
}

fn succeeded(outcome: Option<Result<String, DispatchError>>) -> String {
    outcome
        .expect("the dispatch owns this provider")
        .expect("the arm answers")
}

fn only_call(handler: &Recorder) -> CompletionCall {
    let mut calls = handler.calls();
    assert_eq!(calls.len(), 1, "the arm calls the handler once");
    calls.remove(0)
}

fn authorization(call: &CompletionCall) -> Option<String> {
    call.headers
        .as_ref()
        .and_then(|headers| headers.get("Authorization"))
        .cloned()
}

fn arm_fixture(passed: Option<&str>, section: SectionFixture) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    fixture.configure(section);
    fixture
}

fn arm_call(tag: ChatConfigTag, fixture: &Fixture) -> CompletionCall {
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, fixture, &handler);
    succeeded(outcome);
    only_call(&handler)
}

#[rstest]
#[case::anthropic(ChatConfigTag::Anthropic)]
#[case::openai_gpt(ChatConfigTag::OpenAiGpt)]
fn dispatch_leaves_other_providers_alone(#[case] tag: ChatConfigTag) {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, &fixture, &handler);
    assert!(outcome.is_none(), "{tag:?} belongs to another arm");
    assert!(handler.calls().is_empty());
}

#[rstest]
#[case::hosted_vllm(
    ChatConfigTag::HostedVllmChat,
    SelfHostedConfig::HostedVllm,
    "hosted_vllm"
)]
#[case::oobabooga(ChatConfigTag::Oobabooga, SelfHostedConfig::Oobabooga, "oobabooga")]
#[case::ollama(ChatConfigTag::Ollama, SelfHostedConfig::Ollama, "ollama")]
#[case::ollama_chat(ChatConfigTag::OllamaChat, SelfHostedConfig::OllamaChat, "ollama_chat")]
#[case::petals(ChatConfigTag::Petals, SelfHostedConfig::Petals, "petals")]
#[case::triton(ChatConfigTag::Triton, SelfHostedConfig::Triton, "triton")]
fn dispatch_routes_each_self_hosted_provider_to_its_arm(
    #[case] tag: ChatConfigTag,
    #[case] config: SelfHostedConfig,
    #[case] provider: &str,
) {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://host:8000"));
    fixture.provider = String::from(provider);
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(tag, &fixture, &handler);
    assert_eq!(succeeded(outcome), "reply", "{tag:?}");
    let call = only_call(&handler);
    assert_eq!(call.config, config);
    assert_eq!(call.custom_llm_provider, provider);
}

#[test]
fn dispatch_answers_vllm_with_the_engine_error() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Vllm, &fixture, &handler);
    let error = outcome
        .expect("the dispatch owns vllm")
        .expect_err("the engine cannot run");
    assert_eq!(error.to_string(), "No module named 'vllm'");
    assert_eq!(error.status(), ENGINE_ERROR.status());
    let DispatchError::Vllm(inner) = error else {
        panic!("vllm answers the engine error");
    };
    assert!(matches!(
        *inner,
        VllmError::Engine {
            status_code: 400,
            ..
        }
    ));
    assert!(handler.calls().is_empty());
}

fn hosted_vllm_base(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    arm_call(ChatConfigTag::HostedVllmChat, &arm_fixture(passed, section)).api_base
}

#[test]
fn hosted_vllm_base_falls_from_the_passed_value_to_the_environment_then_litellm() {
    let every_source = SectionFixture::default()
        .hosted_vllm_api_base(Some("http://env"))
        .api_base(Some("http://litellm"));
    assert_eq!(
        hosted_vllm_base(Some("http://passed"), every_source),
        Some(String::from("http://passed"))
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture::default()
                .hosted_vllm_api_base(Some("http://env"))
                .api_base(Some("http://litellm"))
        ),
        Some(String::from("http://env")),
        "the environment wins over litellm.api_base"
    );
    assert_eq!(
        hosted_vllm_base(
            Some(""),
            SectionFixture::default().hosted_vllm_api_base(Some("http://env"))
        ),
        Some(String::from("http://env")),
        "an empty passed base falls through"
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture::default().api_base(Some("http://litellm"))
        ),
        Some(String::from("http://litellm"))
    );
    assert_eq!(hosted_vllm_base(None, SectionFixture::default()), None);
}

fn hosted_vllm_key(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    let mut fixture = arm_fixture(passed, section);
    fixture.api_key = passed.map(secret);
    secret_text(
        arm_call(ChatConfigTag::HostedVllmChat, &fixture)
            .api_key
            .as_ref(),
    )
}

#[test]
fn hosted_vllm_key_falls_from_the_passed_value_to_the_environment() {
    assert_eq!(
        hosted_vllm_key(
            Some("sk-passed"),
            SectionFixture::default()
                .hosted_vllm_api_key(Some("sk-env"))
                .api_key(Some("sk-litellm"))
        ),
        Some(String::from("sk-passed"))
    );
    assert_eq!(
        hosted_vllm_key(
            None,
            SectionFixture::default()
                .hosted_vllm_api_key(Some("sk-env"))
                .api_key(Some("sk-litellm"))
        ),
        Some(String::from("sk-env"))
    );
    assert_eq!(
        hosted_vllm_key(
            Some(""),
            SectionFixture::default().hosted_vllm_api_key(Some("sk-env"))
        ),
        Some(String::from("sk-env")),
        "an empty passed key falls through"
    );
    assert_eq!(
        hosted_vllm_key(None, SectionFixture::default().api_key(Some("sk-litellm"))),
        None,
        "litellm.api_key does not reach the hosted_vllm arm"
    );
}

#[test]
fn hosted_vllm_logs_the_response_after_a_successful_call() {
    let fixture = Fixture::new();
    let handler = Recorder::succeeding();
    let (outcome, state) = run(ChatConfigTag::HostedVllmChat, &fixture, &handler);
    assert_eq!(succeeded(outcome), "reply");
    let details = &state.model_call_details;
    assert_eq!(details.get("original_response"), Some(&LogJson::Null));
    assert_eq!(
        details.get("log_event_type"),
        Some(&LogJson::Text(String::from("post_api_call")))
    );
    let Some(LogJson::List(input)) = details.get("input") else {
        panic!("the post_call log holds the message list");
    };
    assert_eq!(input.len(), 1);
}

#[test]
fn hosted_vllm_leaves_the_log_alone_when_the_handler_fails() {
    let fixture = Fixture::new();
    let handler = Recorder::failing(503);
    let (outcome, state) = run(ChatConfigTag::HostedVllmChat, &fixture, &handler);
    let error = outcome
        .expect("the dispatch owns hosted_vllm")
        .expect_err("the handler fails");
    assert_eq!(error.status(), Some(503));
    assert!(matches!(
        error,
        DispatchError::Handler(HandlerError::Upstream { status_code: 503 })
    ));
    assert!(state.model_call_details.get("original_response").is_none());
}

#[test]
fn oobabooga_drops_the_key_and_headers() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("oobabooga");
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = Some(secret("sk-passed"));
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Extra"),
        String::from("one"),
    )]));
    fixture.configure(
        SectionFixture::default()
            .api_base(Some("http://litellm"))
            .api_key(Some("sk-litellm")),
    );
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Oobabooga, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(call.api_base, Some(String::from("http://passed")));
    assert!(call.api_key.is_none());
    assert!(call.headers.is_none());
    assert_eq!(call.custom_llm_provider, "oobabooga");
}

fn ollama_base(passed: Option<&str>, section: SectionFixture) -> String {
    arm_call(ChatConfigTag::Ollama, &arm_fixture(passed, section))
        .api_base
        .expect("the ollama arm always resolves a base")
}

#[test]
fn ollama_base_prefers_litellm_then_the_passed_base_then_the_environment() {
    assert_eq!(
        ollama_base(
            Some("http://passed"),
            SectionFixture::default()
                .ollama_api_base(Some("http://env"))
                .api_base(Some("http://litellm"))
        ),
        "http://litellm"
    );
    assert_eq!(
        ollama_base(
            Some("http://passed"),
            SectionFixture::default().ollama_api_base(Some("http://env"))
        ),
        "http://passed"
    );
    assert_eq!(
        ollama_base(
            None,
            SectionFixture::default().ollama_api_base(Some("http://env"))
        ),
        "http://env"
    );
    assert_eq!(
        ollama_base(
            Some(""),
            SectionFixture::default().ollama_api_base(Some("http://env"))
        ),
        "http://env",
        "an empty passed base falls through"
    );
    assert_eq!(
        ollama_base(None, SectionFixture::default()),
        "http://localhost:11434"
    );
}

#[rstest]
#[case::passed_key(Some("sk-passed"), Some("sk-env"), Some("sk-passed"))]
#[case::environment_key(None, Some("sk-env"), Some("sk-env"))]
#[case::empty_passed_key(Some(""), Some("sk-env"), Some("sk-env"))]
#[case::no_key(None, None, None)]
fn ollama_takes_the_key_from_the_passed_value_or_the_environment(
    #[case] passed: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = Fixture::new();
    fixture.api_key = passed.map(secret);
    fixture.configure(
        SectionFixture::default()
            .ollama_api_key(environment)
            .api_key(Some("sk-litellm")),
    );
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
    assert_eq!(
        authorization(&call),
        expected.map(|key| format!("Bearer {key}"))
    );
}

#[test]
fn ollama_keeps_a_given_authorization_header() {
    let mut fixture = Fixture::new();
    fixture.api_key = Some(secret("sk-passed"));
    fixture.headers = Some(BTreeMap::from([
        (String::from("Authorization"), String::from("Bearer given")),
        (String::from("X-Extra"), String::from("one")),
    ]));
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(authorization(&call), Some(String::from("Bearer given")));
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Extra")),
        Some(&String::from("one"))
    );
}

#[test]
fn ollama_passes_the_given_headers_without_a_key() {
    let mut fixture = Fixture::new();
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Extra"),
        String::from("one"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert!(authorization(&call).is_none());
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Extra")),
        Some(&String::from("one"))
    );
}

#[test]
fn ollama_ignores_litellm_api_key() {
    let mut fixture = Fixture::new();
    fixture.configure(SectionFixture::default().api_key(Some("sk-litellm")));
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert!(call.api_key.is_none());
    assert!(call.headers.is_none());
}

#[rstest]
#[case::passed_key(
    Some("sk-passed"),
    Some("sk-ollama"),
    Some("sk-env"),
    Some("sk-litellm"),
    Some("sk-passed")
)]
#[case::ollama_key(
    None,
    Some("sk-ollama"),
    Some("sk-env"),
    Some("sk-litellm"),
    Some("sk-ollama")
)]
#[case::environment_key(None, None, Some("sk-env"), Some("sk-litellm"), Some("sk-env"))]
#[case::litellm_key(None, None, None, Some("sk-litellm"), Some("sk-litellm"))]
#[case::empty_passed_and_ollama_keys(Some(""), Some(""), Some("sk-env"), None, Some("sk-env"))]
#[case::no_key(None, None, None, None, None)]
fn ollama_chat_key_falls_through_ollama_key_the_environment_and_litellm(
    #[case] passed: Option<&str>,
    #[case] ollama_key: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] litellm: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = Fixture::new();
    fixture.api_key = passed.map(secret);
    fixture.provider = String::from("ollama_chat");
    fixture.configure(
        SectionFixture::default()
            .ollama_key(ollama_key)
            .ollama_api_key(environment)
            .api_key(litellm),
    );
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::OllamaChat, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
    assert_eq!(
        authorization(&call),
        expected.map(|key| format!("Bearer {key}"))
    );
    assert_eq!(call.custom_llm_provider, "ollama_chat");
}

fn ollama_chat_base(passed: Option<&str>, section: SectionFixture) -> String {
    let mut fixture = arm_fixture(passed, section);
    fixture.provider = String::from("ollama_chat");
    arm_call(ChatConfigTag::OllamaChat, &fixture)
        .api_base
        .expect("the ollama_chat arm always resolves a base")
}

#[test]
fn ollama_chat_reads_the_ollama_base_chain() {
    assert_eq!(
        ollama_chat_base(
            Some("http://passed"),
            SectionFixture::default()
                .ollama_api_base(Some("http://env"))
                .api_base(Some("http://litellm"))
        ),
        "http://litellm"
    );
    assert_eq!(
        ollama_chat_base(
            None,
            SectionFixture::default().ollama_api_base(Some("http://env"))
        ),
        "http://env",
        "the environment base reaches ollama_chat too"
    );
    assert_eq!(
        ollama_chat_base(None, SectionFixture::default()),
        "http://localhost:11434"
    );
}

// the caller's own request values, every one of which the triton arm must carry through
fn triton_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("triton");
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = Some(secret("sk-passed"));
    fixture.headers = Some(BTreeMap::from([(
        String::from("X-Extra"),
        String::from("one"),
    )]));
    fixture.optional_params.insert(
        String::from("temperature"),
        Json::Number(Number::from(1_i32)),
    );
    fixture
        .litellm_params
        .insert(String::from("deployment"), Json::Text(String::from("one")));
    fixture.acompletion = true;
    fixture.timeout = Some(Duration::from_secs(12));
    fixture.configure(SectionFixture::default().api_base(Some("http://litellm")));
    fixture
}

#[test]
fn triton_prefers_litellm_api_base_and_keeps_the_context_values() {
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Triton, &triton_fixture(), &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(call.api_base, Some(String::from("http://litellm")));
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        Some(String::from("sk-passed"))
    );
    assert_eq!(
        call.headers
            .as_ref()
            .and_then(|headers| headers.get("X-Extra")),
        Some(&String::from("one"))
    );
    assert_eq!(call.model, "model");
    assert_eq!(call.messages.len(), 1);
    assert!(call.acompletion);
    assert_eq!(call.timeout, Some(Duration::from_secs(12)));
    assert_eq!(
        call.optional_params.get("temperature"),
        Some(&Json::Number(Number::from(1_i32)))
    );
    assert_eq!(
        call.litellm_params.get("deployment"),
        Some(&Json::Text(String::from("one")))
    );
    assert_eq!(call.custom_llm_provider, "triton");
}

fn petals_call(
    passed: Option<&str>,
    section: SectionFixture,
    optional_params: &[(&str, Json)],
) -> (Result<String, DispatchError>, Recorder) {
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    for (name, value) in optional_params {
        fixture
            .optional_params
            .insert((*name).to_owned(), value.clone());
    }
    fixture.configure(section);
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Petals, &fixture, &handler);
    let result = outcome.expect("the dispatch owns petals");
    (result, handler)
}

#[test]
fn petals_takes_the_stream_out_of_the_optional_params() {
    let (first, first_handler) = petals_call(
        Some("http://passed"),
        SectionFixture::default().api_base(Some("http://litellm")),
        &[
            ("stream", Json::Bool(true)),
            ("temperature", Json::Number(Number::from(1_i32))),
        ],
    );
    assert_eq!(first.expect("the arm answers"), "reply");
    let call = only_call(&first_handler);
    assert_eq!(call.api_base, Some(String::from("http://passed")));
    assert_eq!(call.stream, Some(true));
    assert!(call.optional_params.get("stream").is_none());
    assert_eq!(
        call.optional_params.get("temperature"),
        Some(&Json::Number(Number::from(1_i32)))
    );
    assert_eq!(call.custom_llm_provider, "petals");

    let (null_stream, null_handler) = petals_call(
        Some("http://passed"),
        SectionFixture::default(),
        &[("stream", Json::Null)],
    );
    null_stream.expect("the arm answers");
    assert_eq!(only_call(&null_handler).stream, None);

    let (no_stream, no_handler) = petals_call(
        Some("http://passed"),
        SectionFixture::default(),
        &[("stream", Json::Bool(false))],
    );
    no_stream.expect("the arm answers");
    assert_eq!(only_call(&no_handler).stream, Some(false));
}

#[test]
fn petals_takes_litellm_api_base_when_no_passed_base_arrives() {
    let (result, handler) = petals_call(
        None,
        SectionFixture::default().api_base(Some("http://litellm")),
        &[],
    );
    result.expect("the arm answers");
    assert_eq!(
        only_call(&handler).api_base,
        Some(String::from("http://litellm"))
    );
}

#[test]
fn petals_drops_the_callers_headers_and_key() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = Some(secret("sk-caller"));
    fixture.headers = Some(BTreeMap::from([(
        String::from("Authorization"),
        String::from("Basic caller"),
    )]));
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Petals, &fixture, &handler);
    outcome
        .expect("the dispatch owns petals")
        .expect("the arm answers");
    let call = only_call(&handler);
    assert_eq!(call.headers, None);
    assert!(call.api_key.is_none(), "petals receives no key");
}

#[test]
fn petals_rejects_a_stream_that_is_not_boolean() {
    let (result, handler) = petals_call(
        None,
        SectionFixture::default(),
        &[("stream", Json::Text(String::from("yes")))],
    );
    let error = result.expect_err("the stream flag is invalid");
    assert_eq!(error.status(), Some(400));
    assert!(matches!(error, DispatchError::StreamFlag));
    assert!(
        handler.calls().is_empty(),
        "the invalid flag wins over the missing base"
    );
}

#[test]
fn petals_answers_with_the_engine_error_without_a_base() {
    let (result, handler) = petals_call(None, SectionFixture::default(), &[]);
    let error = result.expect_err("the local engine cannot run");
    assert_eq!(error.to_string(), ENGINE_MISSING_MESSAGE);
    assert_eq!(error.status(), Some(400));
    assert!(matches!(
        error,
        DispatchError::Petals(PetalsSetupError::Engine {
            status_code: 400,
            ..
        })
    ));
    assert!(handler.calls().is_empty());
}

fn lemonade_key(passed: Option<&str>, base: Option<&str>, section: SectionFixture) -> String {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("lemonade");
    fixture.api_base = base.map(str::to_owned);
    fixture.api_key = passed.map(secret);
    fixture.configure(section);
    let handler = Recorder::succeeding();
    complete_lemonade(&chat_context(&fixture), &handler)
        .expect("the lemonade arm answers without a log");
    let call = only_call(&handler);
    assert_eq!(call.config, SelfHostedConfig::Lemonade);
    assert_eq!(call.custom_llm_provider, "lemonade");
    secret_text(call.api_key.as_ref()).expect("the arm always sends a key")
}

fn every_lemonade_source() -> SectionFixture {
    SectionFixture::default()
        .lemonade_key(Some("sk-lemon"))
        .lemonade_api_key(Some("sk-env"))
        .api_key(Some("sk-litellm"))
}

#[test]
fn lemonade_reads_the_key_chain_when_no_base_is_passed() {
    assert_eq!(
        lemonade_key(Some("sk-passed"), None, every_lemonade_source()),
        "sk-passed"
    );
    assert_eq!(
        lemonade_key(None, None, every_lemonade_source()),
        "sk-lemon"
    );
    assert_eq!(
        lemonade_key(
            Some(""),
            None,
            SectionFixture::default().lemonade_api_key(Some("sk-env"))
        ),
        "sk-env"
    );
    assert_eq!(
        lemonade_key(
            None,
            None,
            SectionFixture::default().api_key(Some("sk-litellm"))
        ),
        "lemonade",
        "litellm.api_key does not reach the lemonade arm"
    );
    assert_eq!(
        lemonade_key(None, None, SectionFixture::default()),
        "lemonade"
    );
}

#[test]
fn lemonade_sends_the_default_key_when_a_base_arrives_without_one() {
    assert_eq!(
        lemonade_key(
            None,
            Some("http://host:8000/api/v1"),
            every_lemonade_source()
        ),
        "lemonade"
    );
    assert_eq!(
        lemonade_key(
            Some(""),
            Some("http://host:8000/api/v1"),
            every_lemonade_source()
        ),
        "lemonade"
    );
    assert_eq!(
        lemonade_key(
            Some("sk-passed"),
            Some("http://host:8000/api/v1"),
            every_lemonade_source()
        ),
        "sk-passed"
    );
    assert_eq!(
        lemonade_key(None, Some(""), every_lemonade_source()),
        "lemonade",
        "an explicit empty base still counts as passed"
    );
}
