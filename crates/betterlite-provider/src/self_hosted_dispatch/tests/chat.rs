use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json as LogJson;
use betterlite_prompts::arguments::json::Json;

use crate::petals::error::{ENGINE_MISSING_MESSAGE, PetalsSetupError};
use crate::self_hosted_dispatch::call::{CompletionCall, SelfHostedConfig};
use crate::self_hosted_dispatch::chat::{complete_lemonade, dispatch};
use crate::self_hosted_dispatch::context::CallLogging;
use crate::self_hosted_dispatch::error::DispatchError;
use crate::self_hosted_dispatch::handler::HandlerError;
use crate::vllm::completion::ENGINE_ERROR;
use crate::vllm::error::VllmError;

use super::support::{Fixture, Recorder, SectionFixture, logging, redactor, secret, secret_text};

fn run(
    tag: ChatConfigTag,
    fixture: &Fixture,
    handler: &Recorder,
) -> (Option<Result<String, DispatchError>>, LoggingState) {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let outcome = {
        let mut log = CallLogging {
            state: &mut state,
            env: &env,
        };
        dispatch(tag, &fixture.chat_context(), handler, &mut log)
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

#[test]
fn dispatch_leaves_other_providers_alone() {
    let fixture = Fixture::new();
    for tag in [ChatConfigTag::Anthropic, ChatConfigTag::OpenAiGpt] {
        let handler = Recorder::succeeding();
        let (outcome, _state) = run(tag, &fixture, &handler);
        assert!(outcome.is_none(), "{tag:?} belongs to another arm");
        assert!(handler.calls().is_empty());
    }
}

#[test]
fn dispatch_routes_each_self_hosted_provider_to_its_arm() {
    let cases = [
        (
            ChatConfigTag::HostedVllmChat,
            SelfHostedConfig::HostedVllm,
            "hosted_vllm",
        ),
        (
            ChatConfigTag::Oobabooga,
            SelfHostedConfig::Oobabooga,
            "oobabooga",
        ),
        (ChatConfigTag::Ollama, SelfHostedConfig::Ollama, "ollama"),
        (
            ChatConfigTag::OllamaChat,
            SelfHostedConfig::OllamaChat,
            "ollama_chat",
        ),
        (ChatConfigTag::Petals, SelfHostedConfig::Petals, "petals"),
        (ChatConfigTag::Triton, SelfHostedConfig::Triton, "triton"),
    ];
    for (tag, config, provider) in cases {
        let mut fixture = Fixture::new();
        fixture.api_base = Some(String::from("http://host:8000"));
        let handler = Recorder::succeeding();
        let (outcome, _state) = run(tag, &fixture, &handler);
        assert_eq!(succeeded(outcome), "reply", "{tag:?}");
        let call = only_call(&handler);
        assert_eq!(call.config, config);
        assert_eq!(call.custom_llm_provider, provider);
    }
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
    assert!(matches!(
        error,
        DispatchError::Vllm(VllmError::Engine {
            status_code: 400,
            ..
        })
    ));
    assert!(handler.calls().is_empty());
}

fn hosted_vllm_base(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    fixture.configure(section);
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::HostedVllmChat, &fixture, &handler);
    succeeded(outcome);
    only_call(&handler).api_base
}

#[test]
fn hosted_vllm_base_falls_from_the_passed_value_to_the_environment_then_litellm() {
    let every_source = SectionFixture {
        hosted_vllm_api_base: Some(String::from("http://env")),
        api_base: Some(String::from("http://litellm")),
        ..SectionFixture::default()
    };
    assert_eq!(
        hosted_vllm_base(Some("http://passed"), every_source),
        Some(String::from("http://passed"))
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture {
                hosted_vllm_api_base: Some(String::from("http://env")),
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://env")),
        "the environment wins over litellm.api_base"
    );
    assert_eq!(
        hosted_vllm_base(
            Some(""),
            SectionFixture {
                hosted_vllm_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://env")),
        "an empty passed base falls through"
    );
    assert_eq!(
        hosted_vllm_base(
            None,
            SectionFixture {
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("http://litellm"))
    );
    assert_eq!(hosted_vllm_base(None, SectionFixture::default()), None);
}

fn hosted_vllm_key(passed: Option<&str>, section: SectionFixture) -> Option<String> {
    let mut fixture = Fixture::new();
    fixture.api_key = passed.and_then(secret);
    fixture.configure(section);
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::HostedVllmChat, &fixture, &handler);
    succeeded(outcome);
    secret_text(only_call(&handler).api_key.as_ref())
}

#[test]
fn hosted_vllm_key_falls_from_the_passed_value_to_the_environment() {
    assert_eq!(
        hosted_vllm_key(
            Some("sk-passed"),
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-passed"))
    );
    assert_eq!(
        hosted_vllm_key(
            None,
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-env"))
    );
    assert_eq!(
        hosted_vllm_key(
            Some(""),
            SectionFixture {
                hosted_vllm_api_key: Some(String::from("sk-env")),
                ..SectionFixture::default()
            }
        ),
        Some(String::from("sk-env")),
        "an empty passed key falls through"
    );
    assert_eq!(
        hosted_vllm_key(
            None,
            SectionFixture {
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            }
        ),
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
    fixture.api_key = secret("sk-passed");
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    fixture.configure(SectionFixture {
        api_base: Some(String::from("http://litellm")),
        api_key: Some(String::from("sk-litellm")),
        ..SectionFixture::default()
    });
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
    let mut fixture = Fixture::new();
    fixture.api_base = passed.map(str::to_owned);
    fixture.configure(section);
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    only_call(&handler)
        .api_base
        .expect("the ollama arm always resolves a base")
}

#[test]
fn ollama_base_prefers_litellm_then_the_passed_base_then_the_environment() {
    assert_eq!(
        ollama_base(
            Some("http://passed"),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                api_base: Some(String::from("http://litellm")),
                ..SectionFixture::default()
            }
        ),
        "http://litellm"
    );
    assert_eq!(
        ollama_base(
            Some("http://passed"),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://passed"
    );
    assert_eq!(
        ollama_base(
            None,
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://env"
    );
    assert_eq!(
        ollama_base(
            Some(""),
            SectionFixture {
                ollama_api_base: Some(String::from("http://env")),
                ..SectionFixture::default()
            }
        ),
        "http://env",
        "an empty passed base falls through"
    );
    assert_eq!(
        ollama_base(None, SectionFixture::default()),
        "http://localhost:11434"
    );
}

#[test]
fn ollama_takes_the_key_from_the_passed_value_or_the_environment() {
    let cases = [
        (Some("sk-passed"), Some("sk-env"), Some("sk-passed")),
        (None, Some("sk-env"), Some("sk-env")),
        (Some(""), Some("sk-env"), Some("sk-env")),
        (None, None, None),
    ];
    for (passed, env, expected) in cases {
        let mut fixture = Fixture::new();
        fixture.api_key = passed.and_then(secret);
        fixture.configure(SectionFixture {
            ollama_api_key: env.map(str::to_owned),
            api_key: Some(String::from("sk-litellm")),
            ..SectionFixture::default()
        });
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
}

#[test]
fn ollama_keeps_a_given_authorization_header() {
    let mut fixture = Fixture::new();
    fixture.api_key = secret("sk-passed");
    fixture.headers = Some(
        [
            (String::from("Authorization"), String::from("Bearer given")),
            (String::from("X-Extra"), String::from("one")),
        ]
        .into(),
    );
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
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
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
    fixture.configure(SectionFixture {
        api_key: Some(String::from("sk-litellm")),
        ..SectionFixture::default()
    });
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Ollama, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert!(call.api_key.is_none());
    assert!(call.headers.is_none());
}

#[test]
fn ollama_chat_key_falls_through_ollama_key_the_environment_and_litellm() {
    let cases = [
        (
            Some("sk-passed"),
            Some("sk-ollama"),
            Some("sk-env"),
            Some("sk-litellm"),
            Some("sk-passed"),
        ),
        (
            None,
            Some("sk-ollama"),
            Some("sk-env"),
            Some("sk-litellm"),
            Some("sk-ollama"),
        ),
        (
            None,
            None,
            Some("sk-env"),
            Some("sk-litellm"),
            Some("sk-env"),
        ),
        (None, None, None, Some("sk-litellm"), Some("sk-litellm")),
        (Some(""), Some(""), Some("sk-env"), None, Some("sk-env")),
        (None, None, None, None, None),
    ];
    for (passed, ollama_key, env, litellm, expected) in cases {
        let mut fixture = Fixture::new();
        fixture.api_key = passed.and_then(secret);
        fixture.provider = String::from("ollama_chat");
        fixture.configure(SectionFixture {
            ollama_key: ollama_key.map(str::to_owned),
            ollama_api_key: env.map(str::to_owned),
            api_key: litellm.map(str::to_owned),
            ..SectionFixture::default()
        });
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
}

#[test]
fn triton_prefers_litellm_api_base_and_keeps_the_context_values() {
    let mut fixture = Fixture::new();
    fixture.provider = String::from("triton");
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = secret("sk-passed");
    fixture.headers = Some([(String::from("X-Extra"), String::from("one"))].into());
    fixture
        .optional_params
        .insert(String::from("temperature"), Json::Number(1.into()));
    fixture
        .litellm_params
        .insert(String::from("deployment"), Json::Text(String::from("one")));
    fixture.acompletion = true;
    fixture.timeout = Some(std::time::Duration::from_secs(12));
    fixture.configure(SectionFixture {
        api_base: Some(String::from("http://litellm")),
        ..SectionFixture::default()
    });
    let handler = Recorder::succeeding();
    let (outcome, _state) = run(ChatConfigTag::Triton, &fixture, &handler);
    succeeded(outcome);
    let call = only_call(&handler);
    assert_eq!(call.api_base, Some(String::from("http://litellm")));
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        Some(String::from("sk-passed"))
    );
    assert_eq!(authorization(&call), Some(String::from("Bearer sk-passed")));
    assert_eq!(call.model, "model");
    assert_eq!(call.messages.len(), 1);
    assert!(call.acompletion);
    assert_eq!(call.timeout, Some(std::time::Duration::from_secs(12)));
    assert_eq!(
        call.optional_params.get("temperature"),
        Some(&Json::Number(1.into()))
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
        SectionFixture {
            api_base: Some(String::from("http://litellm")),
            ..SectionFixture::default()
        },
        &[
            ("stream", Json::Bool(true)),
            ("temperature", Json::Number(1.into())),
        ],
    );
    assert_eq!(first.expect("the arm answers"), "reply");
    let call = only_call(&first_handler);
    assert_eq!(call.api_base, Some(String::from("http://passed")));
    assert_eq!(call.stream, Some(true));
    assert!(call.optional_params.get("stream").is_none());
    assert_eq!(
        call.optional_params.get("temperature"),
        Some(&Json::Number(1.into()))
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

    let (litellm_base, litellm_handler) = petals_call(
        None,
        SectionFixture {
            api_base: Some(String::from("http://litellm")),
            ..SectionFixture::default()
        },
        &[],
    );
    litellm_base.expect("the arm answers");
    assert_eq!(
        only_call(&litellm_handler).api_base,
        Some(String::from("http://litellm"))
    );
}

#[test]
fn petals_drops_the_callers_headers_and_key() {
    let mut fixture = Fixture::new();
    fixture.api_base = Some(String::from("http://passed"));
    fixture.api_key = secret("sk-caller");
    fixture.headers = Some(std::collections::BTreeMap::from([(
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
    fixture.api_key = passed.and_then(secret);
    fixture.configure(section);
    let handler = Recorder::succeeding();
    complete_lemonade(&fixture.chat_context(), &handler)
        .expect("the lemonade arm answers without a log");
    let call = only_call(&handler);
    assert_eq!(call.config, SelfHostedConfig::Lemonade);
    assert_eq!(call.custom_llm_provider, "lemonade");
    secret_text(call.api_key.as_ref()).expect("the arm always sends a key")
}

#[test]
fn lemonade_reads_the_key_chain_when_no_base_is_passed() {
    let every_key = || SectionFixture {
        lemonade_key: Some(String::from("sk-lemon")),
        lemonade_api_key: Some(String::from("sk-env")),
        api_key: Some(String::from("sk-litellm")),
        ..SectionFixture::default()
    };
    assert_eq!(
        lemonade_key(Some("sk-passed"), None, every_key()),
        "sk-passed"
    );
    assert_eq!(lemonade_key(None, None, every_key()), "sk-lemon");
    assert_eq!(
        lemonade_key(
            Some(""),
            None,
            SectionFixture {
                lemonade_api_key: Some(String::from("sk-env")),
                ..SectionFixture::default()
            }
        ),
        "sk-env"
    );
    assert_eq!(
        lemonade_key(
            None,
            None,
            SectionFixture {
                api_key: Some(String::from("sk-litellm")),
                ..SectionFixture::default()
            }
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
    let every_key = || SectionFixture {
        lemonade_key: Some(String::from("sk-lemon")),
        lemonade_api_key: Some(String::from("sk-env")),
        api_key: Some(String::from("sk-litellm")),
        ..SectionFixture::default()
    };
    assert_eq!(
        lemonade_key(None, Some("http://host:8000/api/v1"), every_key()),
        "lemonade"
    );
    assert_eq!(
        lemonade_key(Some(""), Some("http://host:8000/api/v1"), every_key()),
        "lemonade"
    );
    assert_eq!(
        lemonade_key(
            Some("sk-passed"),
            Some("http://host:8000/api/v1"),
            every_key()
        ),
        "sk-passed"
    );
    assert_eq!(
        lemonade_key(None, Some(""), every_key()),
        "lemonade",
        "an explicit empty base still counts as passed"
    );
}
