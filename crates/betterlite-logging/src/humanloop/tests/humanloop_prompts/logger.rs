use betterlite_models::prompt_contracts::PromptIgnoreFlags;
use rstest::rstest;

use super::support::{PROMPT_BODY, fake_clock, json, logger, params, replying, request};
use crate::humanloop::compile::CompileError;
use crate::humanloop::logger::{ChatCompletionPrompt, DynamicKeyError, HumanloopError};

#[rstest]
#[case::no_dynamic_key("{}")]
#[case::null_dynamic_key(r#"{"humanloop_api_key": null}"#)]
#[case::empty_dynamic_key(r#"{"humanloop_api_key": ""}"#)]
fn a_missing_api_key_falls_back_to_the_parent_behavior(#[case] dynamic: &str) {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, None);
    let dynamic_params = json(dynamic);
    let outcome = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), None, &dynamic_params))
        .expect("fallback");
    assert_eq!(
        outcome,
        ChatCompletionPrompt {
            model: "humanloop/gpt-3.5".to_owned(),
            messages: vec![json(r#"{"role": "user", "content": "caller message"}"#)],
            non_default_params: params(r#"{"temperature": 0.9, "top_p": 0.5}"#),
        }
    );
    assert!(recorded.lock().expect("lock").is_empty());
}

#[rstest]
#[case::with_a_key(Some("hl-key"))]
#[case::without_a_key(None)]
fn a_missing_prompt_id_is_an_error(#[case] api_key: Option<&str>) {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, api_key);
    let dynamic = json("{}");
    let error = logger
        .get_chat_completion_prompt(request(None, None, &dynamic))
        .expect_err("prompt id");
    assert!(matches!(error, HumanloopError::MissingPromptId));
    assert_eq!(
        error.to_string(),
        "prompt_id is required for Humanloop integration"
    );
    assert!(recorded.lock().expect("lock").is_empty());
}

#[test]
fn the_fetched_prompt_replaces_messages_model_and_parameters() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let variables = params(r#"{"name": "Bob"}"#);
    let outcome = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&variables), &dynamic))
        .expect("prompt");
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.messages,
        vec![
            json(r#"{"role": "system", "content": "You help Bob."}"#),
            json(r#"{"role": "user", "content": "Say hello"}"#),
        ]
    );
    assert_eq!(
        outcome.non_default_params,
        params(r#"{"temperature": 0.2, "top_p": 0.5, "max_tokens": 64}"#)
    );
    let fetches = recorded.lock().expect("lock");
    assert_eq!(
        fetches[0].headers.get("X-Api-Key").map(String::as_str),
        Some("hl-key")
    );
}

#[test]
fn a_dynamic_api_key_wins_over_the_configured_one() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("env-key"));
    let dynamic = json(r#"{"humanloop_api_key": "request-key"}"#);
    let variables = params(r#"{"name": "Bob"}"#);
    logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&variables), &dynamic))
        .expect("prompt");
    let fetches = recorded.lock().expect("lock");
    assert_eq!(
        fetches[0].headers.get("X-Api-Key").map(String::as_str),
        Some("request-key")
    );
}

#[test]
fn a_dynamic_api_key_enables_the_integration_without_a_configured_one() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, None);
    let dynamic = json(r#"{"humanloop_api_key": "request-key"}"#);
    let variables = params(r#"{"name": "Bob"}"#);
    let outcome = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&variables), &dynamic))
        .expect("prompt");
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
}

#[test]
fn a_dynamic_api_key_that_is_not_a_string_is_an_error() {
    let clock = fake_clock();
    let (transport, _recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("env-key"));
    let dynamic = json(r#"{"humanloop_api_key": 7}"#);
    let error = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), None, &dynamic))
        .expect_err("key type");
    assert!(matches!(
        error,
        HumanloopError::DynamicKey(DynamicKeyError::NotAString)
    ));
    assert_eq!(error.to_string(), "humanloop_api_key must be a string");
}

#[rstest]
#[case::neither(false, false)]
#[case::model_only(true, false)]
#[case::params_only(false, true)]
#[case::both(true, true)]
fn the_ignore_prompt_manager_arguments_are_not_honored(
    #[case] ignore_model: bool,
    #[case] ignore_optional_params: bool,
) {
    let clock = fake_clock();
    let (transport, _recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let variables = params(r#"{"name": "Bob"}"#);
    let mut ignoring = request(Some("pr_abc123"), Some(&variables), &dynamic);
    ignoring.ignore_flags = PromptIgnoreFlags {
        model: ignore_model,
        optional_params: ignore_optional_params,
    };
    let outcome = logger.get_chat_completion_prompt(ignoring).expect("prompt");
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.non_default_params,
        params(r#"{"temperature": 0.2, "top_p": 0.5, "max_tokens": 64}"#)
    );
}

#[rstest]
#[case::prefix("humanloop/gpt-3.5", "gpt-3.5")]
#[case::no_prefix("gpt-3.5", "gpt-3.5")]
#[case::everywhere("a/humanloop/b/humanloop/c", "a/b/c")]
fn without_a_prompt_model_the_caller_model_loses_the_integration_prefix(
    #[case] caller_model: &str,
    #[case] expected: &str,
) {
    let clock = fake_clock();
    let (transport, _recorded) = replying(200, r#"{"model": null, "template": []}"#);
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let mut by_model = request(Some("pr_abc123"), None, &dynamic);
    by_model.model = caller_model;
    let outcome = logger.get_chat_completion_prompt(by_model).expect("prompt");
    assert_eq!(outcome.model, expected);
    assert_eq!(outcome.messages, Vec::new());
    assert_eq!(
        outcome.non_default_params,
        params(r#"{"temperature": 0.9, "top_p": 0.5}"#)
    );
}

#[test]
fn the_cached_template_keeps_the_first_compilation() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let bob = params(r#"{"name": "Bob"}"#);
    let alice = params(r#"{"name": "Alice"}"#);
    logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&bob), &dynamic))
        .expect("first");
    let second = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&alice), &dynamic))
        .expect("second");
    assert_eq!(
        second.messages[0],
        json(r#"{"role": "system", "content": "You help Bob."}"#)
    );
    assert_eq!(recorded.lock().expect("lock").len(), 1);
}

#[test]
fn a_missing_variable_fails_the_call_after_the_prompt_is_cached() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let error = logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), None, &dynamic))
        .expect_err("variable");
    assert!(matches!(
        error,
        HumanloopError::Compile(CompileError::MissingVariable { .. })
    ));
    let bob = params(r#"{"name": "Bob"}"#);
    logger
        .get_chat_completion_prompt(request(Some("pr_abc123"), Some(&bob), &dynamic))
        .expect("retry");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
}

#[test]
fn an_api_error_reaches_the_caller() {
    let clock = fake_clock();
    let (transport, _recorded) = replying(404, "no such prompt");
    let logger = logger(transport, &clock, Some("hl-key"));
    let dynamic = json("{}");
    let error = logger
        .get_chat_completion_prompt(request(Some("pr_missing"), None, &dynamic))
        .expect_err("api");
    assert_eq!(
        error.to_string(),
        "Error getting prompt from Humanloop: no such prompt"
    );
}
