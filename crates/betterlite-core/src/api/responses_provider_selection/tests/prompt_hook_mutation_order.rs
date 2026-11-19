use betterlite_models::controls::json_value::JsonValue;

use super::{
    HookBehavior, capabilities_configs, configs, hook_answer, json, manager, parameters,
    resolution, resolver,
};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::prompting::{
    CARRY_UNMATCHED_MESSAGE_POINTS, PromptError, PromptOutcome, PromptRequest,
    apply_prompt_management,
};
use crate::api::responses_provider_selection::provider_config::ASYNC_PROMPT_MERGED_PARAMS;

fn run_async(value: JsonValue) -> Result<PromptOutcome, PromptError> {
    let lookup = configs(&[]);
    let resolver = resolver(&[]);
    let mut kwargs = parameters(&[(ASYNC_PROMPT_MERGED_PARAMS, value)]);
    let mut local_vars = ParameterMap::new();
    apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        None,
        &lookup,
        &resolver,
    )
}

#[test]
fn the_hook_receives_the_request_and_its_answers_land_in_the_locals() {
    let lookup = capabilities_configs();
    let resolver = resolver(&[]);
    let answer = hook_answer(
        "gpt-4o",
        &[json(r#"{"role":"user","content":"merged"}"#)],
        &[("temperature", json("0.2"))],
    );
    let hook = manager(HookBehavior::Answer(answer));
    let mut kwargs = parameters(&[
        ("prompt_id", json(r#""p-1""#)),
        ("prompt_label", json(r#""prod""#)),
        ("prompt_version", json("2")),
        ("prompt_variables", json(r#"{"name":"Ada"}"#)),
        ("model_info", json(r#"{"id":"dep-1"}"#)),
        ("temperature", json("0.7")),
    ]);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("the hook answer applies");
    assert_eq!(
        outcome.input,
        json(r#"[{"role":"user","content":"merged"}]"#)
    );
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(outcome.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        outcome.deployment_model_info,
        Some(json(r#"{"id":"dep-1"}"#))
    );
    assert_eq!(local_vars.get("model"), Some(&json(r#""gpt-4o""#)));
    assert_eq!(local_vars.get("temperature"), Some(&json("0.2")));
    assert_eq!(
        local_vars.get("custom_llm_provider"),
        Some(&json(r#""openai""#))
    );
    assert_eq!(
        local_vars.get("input"),
        Some(&json(r#"[{"role":"user","content":"merged"}]"#))
    );
    let requests = hook.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].prompt_id.as_deref(), Some("p-1"));
    assert_eq!(requests[0].non_default_params, 6);
    let calls = hook.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o");
    assert_eq!(
        calls[0].messages,
        vec![json(r#"{"role":"user","content":"hello"}"#)]
    );
    assert_eq!(calls[0].prompt_id.as_deref(), Some("p-1"));
    assert_eq!(calls[0].prompt_label.as_deref(), Some("prod"));
    assert_eq!(calls[0].prompt_version, Some(json("2")));
    assert_eq!(calls[0].prompt_variables, Some(json(r#"{"name":"Ada"}"#)));
    assert_eq!(calls[0].signal, None);
    assert!(resolver.models().is_empty());
    assert!(!kwargs.contains_key(CARRY_UNMATCHED_MESSAGE_POINTS));
}

#[test]
fn a_bridged_request_carries_the_signal_only_during_the_hook() {
    let lookup = configs(&[]);
    let resolver = resolver(&[("gpt-4o", resolution("gpt-4o", "openai"))]);
    let hook = manager(HookBehavior::Answer(hook_answer("gpt-4o", &[], &[])));
    let mut kwargs = ParameterMap::new();
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: None,
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("the hook answer applies");
    let calls = hook.calls();
    assert_eq!(calls[0].signal, Some(json("true")));
    assert!(!kwargs.contains_key(CARRY_UNMATCHED_MESSAGE_POINTS));
    assert_eq!(outcome.custom_llm_provider, None);
}

#[test]
fn a_failing_hook_still_removes_the_signal() {
    let lookup = configs(&[]);
    let resolver = resolver(&[]);
    let hook = manager(HookBehavior::Fail);
    let mut kwargs = ParameterMap::new();
    let mut local_vars = ParameterMap::new();
    let error = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: None,
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect_err("the hook fails");
    assert!(matches!(error, PromptError::Manager(_)));
    assert!(!kwargs.contains_key(CARRY_UNMATCHED_MESSAGE_POINTS));
    let calls = hook.calls();
    assert_eq!(calls[0].signal, Some(json("true")));
}

#[test]
fn a_native_request_keeps_a_pre_existing_signal() {
    let lookup = capabilities_configs();
    let resolver = resolver(&[]);
    let hook = manager(HookBehavior::Answer(hook_answer("gpt-4o", &[], &[])));
    let mut kwargs = parameters(&[(CARRY_UNMATCHED_MESSAGE_POINTS, json("false"))]);
    let mut local_vars = ParameterMap::new();
    apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("the hook answer applies");
    let calls = hook.calls();
    assert_eq!(calls[0].signal, Some(json("false")));
    assert_eq!(
        kwargs.get(CARRY_UNMATCHED_MESSAGE_POINTS),
        Some(&json("false"))
    );
}

#[test]
fn without_a_prompt_manager_nothing_changes() {
    let lookup = configs(&[]);
    let resolver = resolver(&[]);
    let mut kwargs = parameters(&[("model_info", json(r#"{"id":"dep-1"}"#))]);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        None,
        &lookup,
        &resolver,
    )
    .expect("no manager leaves the call unchanged");
    assert_eq!(outcome.input, json(r#""hello""#));
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(outcome.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        outcome.deployment_model_info,
        Some(json(r#"{"id":"dep-1"}"#))
    );
    assert!(local_vars.is_empty());
    assert!(resolver.models().is_empty());
}

#[test]
fn a_skipped_hook_leaves_everything_unchanged() {
    let lookup = configs(&[]);
    let resolver = resolver(&[]);
    let hook = manager(HookBehavior::Skip);
    let mut kwargs = parameters(&[("model_info", json(r#"{"id":"dep-1"}"#))]);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("a skipped hook leaves the call unchanged");
    assert_eq!(outcome.input, json(r#""hello""#));
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(outcome.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(
        outcome.deployment_model_info,
        Some(json(r#"{"id":"dep-1"}"#))
    );
    assert!(local_vars.is_empty());
    assert!(hook.calls().is_empty());
    let requests = hook.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].prompt_id, None);
}

#[test]
fn a_swapped_model_resolves_its_provider_once() {
    let lookup = configs(&[]);
    let resolver = resolver(&[("gpt-4o-mini", resolution("gpt-4o-mini", "openai"))]);
    let hook = manager(HookBehavior::Answer(hook_answer(
        "gpt-4o-mini",
        &[],
        &[("top_p", json("0.9"))],
    )));
    let mut kwargs = ParameterMap::new();
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: None,
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("the swap resolves its provider");
    assert_eq!(outcome.model, "gpt-4o-mini");
    assert_eq!(outcome.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(outcome.deployment_model_info, None);
    assert_eq!(local_vars.get("model"), Some(&json(r#""gpt-4o-mini""#)));
    assert_eq!(
        local_vars.get("custom_llm_provider"),
        Some(&json(r#""openai""#))
    );
    assert_eq!(local_vars.get("top_p"), Some(&json("0.9")));
    assert_eq!(resolver.models(), vec![String::from("gpt-4o-mini")]);
}

#[test]
fn an_async_outcome_skips_the_hook_and_fills_the_locals() {
    let lookup = configs(&[]);
    let resolver = resolver(&[]);
    let hook = manager(HookBehavior::Answer(hook_answer("gpt-4o", &[], &[])));
    let mut kwargs = parameters(&[(
        ASYNC_PROMPT_MERGED_PARAMS,
        json(
            r#"{"merged_optional_params":{"temperature":0.1},"deployment_model_info":{"id":"dep-async"}}"#,
        ),
    )]);
    let mut local_vars = ParameterMap::new();
    let outcome = apply_prompt_management(
        PromptRequest {
            input: json(r#""hello""#),
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: false,
            kwargs: &mut kwargs,
            local_vars: &mut local_vars,
        },
        Some(&hook),
        &lookup,
        &resolver,
    )
    .expect("the async outcome applies");
    assert_eq!(outcome.input, json(r#""hello""#));
    assert_eq!(outcome.model, "gpt-4o");
    assert_eq!(
        outcome.deployment_model_info,
        Some(json(r#"{"id":"dep-async"}"#))
    );
    assert_eq!(local_vars.get("temperature"), Some(&json("0.1")));
    assert!(hook.calls().is_empty());
    assert!(resolver.models().is_empty());
    assert!(!kwargs.contains_key(ASYNC_PROMPT_MERGED_PARAMS));
}

#[test]
fn an_async_outcome_without_deployment_info_is_accepted() {
    let outcome =
        run_async(json(r#"{"merged_optional_params":{}}"#)).expect("the async outcome applies");
    assert_eq!(outcome.deployment_model_info, None);
}

#[test]
fn a_malformed_async_outcome_is_an_error() {
    let not_object = run_async(json("7")).expect_err("a number is not an async outcome");
    assert!(matches!(not_object, PromptError::AsyncOutcome(_)));
    let missing_params = run_async(json(r#"{"deployment_model_info":{"id":"dep"}}"#))
        .expect_err("a missing merged_optional_params is not an async outcome");
    assert!(matches!(missing_params, PromptError::AsyncOutcome(_)));
}
