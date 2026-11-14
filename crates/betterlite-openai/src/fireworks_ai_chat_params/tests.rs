#![cfg(test)]

use std::future::{Future, ready};

use betterlite_context::error::ContextError;
use betterlite_models::entry::ModelFields;
use betterlite_models::fireworks_catalog_metadata::Catalog;
use rstest::rstest;
use serde_json::Value;
use serde_json::value::to_raw_value;

use super::extra_body::map_extra_body_params;
use super::mapping::map_openai_params;
use super::settings::{ContextLengthBehavior, FireworksChatSettings};
use super::supported::supported_openai_params;
use super::{
    EFFORT_KWARG_KEYS, EXTRA_BODY_CONSUMED_PARAMS, FireworksCapabilities, FireworksChatParamsError,
    NIM_VLLM_STRIP_PARAMS,
};
use crate::openai_base_config::Object;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    FunctionCalling,
    ToolChoice,
    Reasoning,
}

#[derive(Debug, Clone, Copy)]
struct TestCapabilities {
    on: &'static [Gate],
}

impl TestCapabilities {
    const fn none() -> Self {
        Self { on: &[] }
    }

    const fn with(on: &'static [Gate]) -> Self {
        Self { on }
    }
}

impl FireworksCapabilities for TestCapabilities {
    fn supports_function_calling(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.on.contains(&Gate::FunctionCalling))
    }

    fn supports_tool_choice(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.on.contains(&Gate::ToolChoice))
    }

    fn supports_reasoning(&mut self, _model: &str) -> impl Future<Output = bool> + Send {
        ready(self.on.contains(&Gate::Reasoning))
    }
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).expect("a JSON value")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("a JSON object")
}

fn fields(info: &str) -> ModelFields {
    let raw = to_raw_value(&value(info)).expect("raw JSON");
    ModelFields::parse(&raw).expect("model fields").0
}

fn model_catalog(entries: &[(&str, &str)]) -> Catalog {
    entries
        .iter()
        .map(|(key, info)| ((*key).to_owned(), fields(info)))
        .collect()
}

fn params(pairs: &[(&str, Value)]) -> Vec<(String, Value)> {
    pairs
        .iter()
        .map(|(name, item)| ((*name).to_owned(), item.clone()))
        .collect()
}

async fn supported(
    capabilities: &mut TestCapabilities,
    catalog: &Catalog,
    model: &str,
) -> Vec<&'static str> {
    supported_openai_params(capabilities, catalog, model).await
}

async fn map_request(
    capabilities: &mut TestCapabilities,
    model: &str,
    pairs: &[(&str, Value)],
) -> Object {
    let mut optional = Object::new();
    map_openai_params(
        capabilities,
        &Catalog::new(),
        model,
        &params(pairs),
        &mut optional,
    )
    .await
    .expect("a mapped request");
    optional
}

async fn extra_body(capabilities: &mut TestCapabilities, model: &str, optional: &Object) -> Object {
    map_extra_body_params(capabilities, model, optional).await
}

const BASE_PARAMS: [&str; 37] = [
    "stream",
    "max_completion_tokens",
    "max_tokens",
    "temperature",
    "top_p",
    "top_k",
    "frequency_penalty",
    "presence_penalty",
    "n",
    "stop",
    "response_format",
    "user",
    "logprobs",
    "prompt_truncate_len",
    "context_length_exceeded_behavior",
    "seed",
    "top_logprobs",
    "min_p",
    "typical_p",
    "repetition_penalty",
    "mirostat_target",
    "mirostat_lr",
    "logit_bias",
    "echo",
    "echo_last",
    "ignore_eos",
    "prompt_cache_key",
    "prompt_cache_isolation_key",
    "raw_output",
    "perf_metrics_in_response",
    "return_token_ids",
    "safe_tokenization",
    "service_tier",
    "speculation",
    "prediction",
    "stream_options",
    "sampling_mask",
];

#[tokio::test]
async fn supported_list_starts_with_the_pinned_order() {
    let mut capabilities = TestCapabilities::none();
    assert_eq!(
        supported(&mut capabilities, &Catalog::new(), "unknown-model").await,
        BASE_PARAMS.to_vec()
    );
}

#[tokio::test]
async fn function_calling_gate_adds_tools_and_parallel_calls() {
    let mut on = TestCapabilities::with(&[Gate::FunctionCalling]);
    let mut expected = BASE_PARAMS.to_vec();
    expected.push("tools");
    expected.push("parallel_tool_calls");
    assert_eq!(
        supported(&mut on, &Catalog::new(), "unknown-model").await,
        expected
    );
    let mut off = TestCapabilities::none();
    assert_eq!(
        supported(&mut off, &Catalog::new(), "unknown-model").await,
        BASE_PARAMS.to_vec()
    );
}

#[tokio::test]
async fn exact_tool_choice_capability_adds_tool_choice() {
    let catalog = model_catalog(&[(
        "fireworks_ai/accounts/fireworks/models/choice-model",
        r#"{"supports_function_calling":false,"supports_tool_choice":true}"#,
    )]);
    let mut capabilities = TestCapabilities::with(&[]);
    let mut found = supported(&mut capabilities, &catalog, "choice-model").await;
    assert_eq!(found.pop(), Some("tool_choice"));
}

#[tokio::test]
async fn exact_tool_choice_false_falls_through_to_the_predicate() {
    let catalog = model_catalog(&[(
        "fireworks_ai/accounts/fireworks/models/choice-model",
        r#"{"supports_function_calling":false,"supports_tool_choice":false}"#,
    )]);
    let mut predicate_on = TestCapabilities::with(&[Gate::ToolChoice]);
    let mut found = supported(&mut predicate_on, &catalog, "choice-model").await;
    assert_eq!(found.pop(), Some("tool_choice"));
    let mut predicate_off = TestCapabilities::none();
    let without = supported(&mut predicate_off, &catalog, "choice-model").await;
    assert!(!without.contains(&"tool_choice"));
}

#[tokio::test]
async fn reasoning_capability_adds_the_three_params() {
    let catalog = model_catalog(&[(
        "fireworks_ai/accounts/fireworks/models/reasoner",
        r#"{"supports_function_calling":false,"supports_reasoning":true}"#,
    )]);
    let mut capabilities = TestCapabilities::with(&[]);
    let found = supported(&mut capabilities, &catalog, "reasoner").await;
    assert_eq!(
        found.get(found.len().saturating_sub(3)..),
        Some(["reasoning_effort", "reasoning_history", "thinking"].as_slice())
    );
}

#[tokio::test]
async fn exact_reasoning_false_falls_through_to_the_predicate() {
    let catalog = model_catalog(&[(
        "fireworks_ai/accounts/fireworks/models/reasoner",
        r#"{"supports_function_calling":false,"supports_reasoning":false}"#,
    )]);
    let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
    let found = supported(&mut capabilities, &catalog, "reasoner").await;
    assert_eq!(
        found.get(found.len().saturating_sub(3)..),
        Some(["reasoning_effort", "reasoning_history", "thinking"].as_slice())
    );
}

#[tokio::test]
async fn reasoning_predicate_alone_adds_the_three_params() {
    let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
    let found = supported(&mut capabilities, &Catalog::new(), "unknown-model").await;
    assert_eq!(
        found.get(found.len().saturating_sub(3)..),
        Some(["reasoning_effort", "reasoning_history", "thinking"].as_slice())
    );
    let mut off = TestCapabilities::none();
    let absent = supported(&mut off, &Catalog::new(), "unknown-model").await;
    assert!(!absent.contains(&"reasoning_history"));
}

#[rstest]
#[case::required(r#""required""#, r#""any""#)]
#[case::auto(r#""auto""#, r#""auto""#)]
#[case::object(
    r#"{"type":"function","function":{"name":"run"}}"#,
    r#"{"type":"function","function":{"name":"run"}}"#
)]
#[case::null("null", "null")]
#[tokio::test]
async fn map_rewrites_tool_choice_required_to_any(#[case] given: &str, #[case] expected: &str) {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("tool_choice", value(given))],
    )
    .await;
    assert_eq!(mapped.get("tool_choice"), Some(&value(expected)));
}

#[tokio::test]
async fn response_format_with_tools_becomes_a_json_tool_call() {
    let mut capabilities = TestCapabilities::with(&[Gate::FunctionCalling]);
    let format = value(
        r#"{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object"}}}"#,
    );
    let tools = value(r#"[{"type":"function","function":{"name":"get_weather"}}]"#);
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("tools", tools), ("response_format", format)],
    )
    .await;
    assert_eq!(
        mapped.get("tools"),
        Some(&value(
            r#"[{"type":"function","function":{"name":"get_weather"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"object"}}}]"#
        ))
    );
    assert_eq!(mapped.get("json_mode"), Some(&Value::Bool(true)));
    assert_eq!(mapped.get("response_format"), None);
}

#[tokio::test]
async fn response_format_prefers_the_response_schema_key() {
    let mut capabilities = TestCapabilities::with(&[Gate::FunctionCalling]);
    let format = value(
        r#"{"response_schema":{"type":"string"},"json_schema":{"schema":{"type":"number"}}}"#,
    );
    let tools = value(r#"[{"type":"function","function":{"name":"given"}}]"#);
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("tools", tools), ("response_format", format)],
    )
    .await;
    assert_eq!(
        mapped.get("tools"),
        Some(&value(
            r#"[{"type":"function","function":{"name":"given"}},{"type":"function","function":{"name":"json_tool_call","parameters":{"type":"string"}}}]"#
        ))
    );
}

#[tokio::test]
async fn response_format_without_tools_is_copied_as_given() {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("response_format", value(r#"{"type":"text"}"#))],
    )
    .await;
    assert_eq!(
        mapped.get("response_format"),
        Some(&value(r#"{"type":"text"}"#))
    );
    let null = map_request(
        &mut capabilities,
        "unknown-model",
        &[("response_format", Value::Null)],
    )
    .await;
    assert_eq!(null.get("response_format"), Some(&Value::Null));
}

#[tokio::test]
async fn empty_response_format_with_tools_changes_nothing() {
    let mut capabilities = TestCapabilities::with(&[Gate::FunctionCalling]);
    let tools = value(r#"[{"type":"function","function":{"name":"only"}}]"#);
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("tools", tools.clone()), ("response_format", value("{}"))],
    )
    .await;
    assert_eq!(mapped.get("tools"), Some(&tools));
    assert_eq!(mapped.get("json_mode"), None);
}

#[tokio::test]
async fn max_completion_tokens_writes_max_tokens() {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("max_completion_tokens", Value::from(128_i64))],
    )
    .await;
    assert_eq!(mapped.get("max_tokens"), Some(&Value::from(128_i64)));
}

#[tokio::test]
async fn the_later_token_limit_key_wins() {
    let mut capabilities = TestCapabilities::none();
    let later = map_request(
        &mut capabilities,
        "unknown-model",
        &[
            ("max_tokens", Value::from(10_i64)),
            ("max_completion_tokens", Value::from(20_i64)),
        ],
    )
    .await;
    assert_eq!(later.get("max_tokens"), Some(&Value::from(20_i64)));
    let earlier = map_request(
        &mut capabilities,
        "unknown-model",
        &[
            ("max_completion_tokens", Value::from(20_i64)),
            ("max_tokens", Value::from(10_i64)),
        ],
    )
    .await;
    assert_eq!(earlier.get("max_tokens"), Some(&Value::from(10_i64)));
}

#[rstest]
#[case::text(r#""high""#, Some(r#""high""#))]
#[case::mapped_low(r#"{"effort":"low"}"#, Some(r#""low""#))]
#[case::mapped_true(r#"{"effort":true}"#, Some(r#""medium""#))]
#[case::mapped_false(r#"{"effort":false}"#, Some(r#""none""#))]
#[case::auto(r#""auto""#, None)]
#[case::mapped_auto(r#"{"effort":"auto"}"#, None)]
#[case::no_effort_key(r#"{"budget":3}"#, None)]
#[case::true_bool("true", Some(r#""medium""#))]
#[case::false_bool("false", Some(r#""none""#))]
#[case::number("1", Some("1"))]
#[case::null("null", None)]
#[case::mapped_null(r#"{"effort":null}"#, None)]
#[tokio::test]
async fn reasoning_effort_forms_map(#[case] given: &str, #[case] expected: Option<&str>) {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("reasoning_effort", value(given))],
    )
    .await;
    assert_eq!(mapped.get("reasoning_effort"), expected.map(value).as_ref());
}

#[tokio::test]
async fn thinking_and_reasoning_effort_together_are_rejected() {
    let mut capabilities = TestCapabilities::none();
    let mut optional = Object::new();
    let error = map_openai_params(
        &mut capabilities,
        &Catalog::new(),
        "thinker",
        &params(&[
            ("thinking", value(r#"{"type":"enabled"}"#)),
            ("reasoning_effort", value(r#""high""#)),
        ]),
        &mut optional,
    )
    .await
    .expect_err("a conflict");
    assert!(matches!(
        &error,
        FireworksChatParamsError::ThinkingConflict { .. }
    ));
    assert_eq!(error.status_code(), Some(400));
    let class = error.class().expect("a client error");
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.model.as_deref(), Some("thinker"));
    assert_eq!(metadata.provider.as_deref(), Some("fireworks_ai"));
    assert_eq!(metadata.code, "400");
}

#[tokio::test]
async fn null_thinking_is_not_a_conflict() {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[
            ("temperature", value("0.5")),
            ("thinking", Value::Null),
            ("reasoning_effort", value(r#""high""#)),
        ],
    )
    .await;
    assert_eq!(mapped.get("reasoning_effort"), Some(&value(r#""high""#)));
}

#[tokio::test]
async fn unsupported_and_null_params_are_not_copied() {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[
            ("modalities", value(r#"["text"]"#)),
            ("top_p", Value::Null),
            ("user", value(r#""someone""#)),
        ],
    )
    .await;
    assert_eq!(mapped.get("modalities"), None);
    assert_eq!(mapped.get("top_p"), None);
    assert_eq!(mapped.get("user"), Some(&value(r#""someone""#)));
}

#[tokio::test]
async fn special_branches_apply_to_models_that_do_not_list_them() {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[
            ("tool_choice", value(r#""required""#)),
            ("max_completion_tokens", Value::from(4_i64)),
            ("reasoning_effort", value(r#""high""#)),
        ],
    )
    .await;
    assert_eq!(mapped.get("tool_choice"), Some(&value(r#""any""#)));
    assert_eq!(mapped.get("max_tokens"), Some(&Value::from(4_i64)));
    assert_eq!(mapped.get("reasoning_effort"), Some(&value(r#""high""#)));
}

#[tokio::test]
async fn config_holds_only_the_values_the_constructor_was_given() {
    assert!(FireworksChatSettings::default().get_config().is_empty());
    let settings = FireworksChatSettings::with_settings(object(
        r#"{"tools":[],"tool_choice":"auto","max_tokens":0,"temperature":0,"top_p":1,"top_k":2,"frequency_penalty":0,"presence_penalty":0,"n":2,"stop":["end"],"response_format":{"type":"text"},"user":"someone","logprobs":3,"reasoning_effort":"high","prompt_truncate_len":9,"context_length_exceeded_behavior":"truncate","tools_null":null}"#,
    ))
    .expect_err("an unknown setting");
    assert!(matches!(
        settings,
        FireworksChatParamsError::Settings { .. }
    ));
    let config = FireworksChatSettings::with_settings(object(
        r#"{"tools":[],"tool_choice":"auto","max_tokens":0,"temperature":0,"top_p":1,"top_k":2,"frequency_penalty":0,"presence_penalty":0,"n":2,"stop":["end"],"response_format":{"type":"text"},"user":"someone","logprobs":3,"reasoning_effort":"high","prompt_truncate_len":9,"context_length_exceeded_behavior":"truncate"}"#,
    ))
    .expect("a settings object")
    .get_config();
    assert_eq!(config.len(), 16);
    assert_eq!(config.get("n"), Some(&Value::from(2_i64)));
    assert_eq!(
        config.get("context_length_exceeded_behavior"),
        Some(&value(r#""truncate""#))
    );
    assert_eq!(config.get("tools"), Some(&value("[]")));
}

#[tokio::test]
async fn config_drops_null_settings() {
    let settings = FireworksChatSettings::with_settings(object(r#"{"temperature":null}"#))
        .expect("a settings object");
    assert!(settings.get_config().is_empty());
    assert_eq!(settings.context_behavior, None);
    let behavior = FireworksChatSettings::with_settings(object(
        r#"{"context_length_exceeded_behavior":"error"}"#,
    ))
    .expect("a settings object");
    assert_eq!(
        behavior.context_behavior,
        Some(ContextLengthBehavior::Error)
    );
}

const PINNED_STRIP_PARAMS: [&str; 16] = [
    "add_generation_prompt",
    "add_special_tokens",
    "allowed_token_ids",
    "bad_words",
    "best_of",
    "continue_final_message",
    "detokenize",
    "guided_decoding_backend",
    "guided_regex",
    "include_reasoning",
    "include_stop_str_in_output",
    "nvext",
    "skip_special_tokens",
    "spaces_between_special_tokens",
    "stop_token_ids",
    "use_beam_search",
];

const TRANSLATED_KEYS: [&str; 5] = [
    "chat_template_kwargs",
    "guided_choice",
    "guided_grammar",
    "guided_json",
    "truncate_prompt_tokens",
];

#[test]
fn constants_match_the_pinned_lists() {
    assert_eq!(
        EFFORT_KWARG_KEYS,
        [
            "enable_thinking",
            "thinking",
            "reasoning_budget",
            "low_effort"
        ]
    );
    assert_eq!(NIM_VLLM_STRIP_PARAMS, PINNED_STRIP_PARAMS);
    let mut consumed: Vec<&str> = PINNED_STRIP_PARAMS.to_vec();
    consumed.extend(TRANSLATED_KEYS);
    consumed.sort_unstable();
    assert_eq!(EXTRA_BODY_CONSUMED_PARAMS.to_vec(), consumed);
}

#[tokio::test]
async fn every_strip_param_is_dropped() {
    let mut capabilities = TestCapabilities::none();
    let extra: Object = NIM_VLLM_STRIP_PARAMS
        .iter()
        .map(|name| ((*name).to_owned(), Value::Bool(true)))
        .chain([(String::from("custom"), Value::Bool(true))])
        .collect();
    let optional = Object::from_iter([(String::from("extra_body"), Value::Object(extra))]);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"extra_body":{"custom":true}}"#)
    );
}

#[tokio::test]
async fn extra_body_that_is_not_a_map_is_returned_unchanged() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":"plain","temperature":1}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        optional
    );
    let absent = object(r#"{"temperature":1}"#);
    assert_eq!(extra_body(&mut capabilities, "m", &absent).await, absent);
}

#[tokio::test]
async fn strip_params_are_dropped_and_remaining_keys_kept() {
    let mut capabilities = TestCapabilities::none();
    let optional =
        object(r#"{"extra_body":{"stop_token_ids":[1],"nvext":true,"custom":{"kept":true}}}"#);
    let mapped = extra_body(&mut capabilities, "m", &optional).await;
    assert_eq!(
        mapped.get("extra_body"),
        Some(&value(r#"{"custom":{"kept":true}}"#))
    );
}

#[tokio::test]
async fn an_emptied_extra_body_is_left_out() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"stop_token_ids":[1]},"temperature":0.5}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"temperature":0.5}"#)
    );
}

#[tokio::test]
async fn truncate_prompt_tokens_is_promoted_to_prompt_truncate_len() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"truncate_prompt_tokens":42,"custom":1}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"prompt_truncate_len":42,"extra_body":{"custom":1}}"#)
    );
}

#[tokio::test]
async fn an_existing_prompt_truncate_len_consumes_truncate_prompt_tokens() {
    let mut capabilities = TestCapabilities::none();
    let in_extra_body =
        object(r#"{"extra_body":{"truncate_prompt_tokens":5,"prompt_truncate_len":9}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_extra_body).await,
        object(r#"{"extra_body":{"prompt_truncate_len":9}}"#)
    );
    let in_optional =
        object(r#"{"prompt_truncate_len":7,"extra_body":{"truncate_prompt_tokens":5}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_optional).await,
        object(r#"{"prompt_truncate_len":7}"#)
    );
}

#[rstest]
#[case::budget(r#"{"reasoning_budget":100}"#, Some("100"))]
#[case::disabled(r#"{"enable_thinking":false}"#, Some(r#""none""#))]
#[case::thinking_key(r#"{"thinking":false}"#, Some(r#""none""#))]
#[case::first_non_bool_skipped(r#"{"enable_thinking":"yes","thinking":false}"#, Some(r#""none""#))]
#[case::fractional_budget(r#"{"reasoning_budget":0.5}"#, Some("0"))]
#[case::fractional_budget_rounds_down(r#"{"reasoning_budget":2.7}"#, Some("2"))]
#[case::negative_budget(r#"{"reasoning_budget":-1}"#, None)]
#[case::negative_float_budget(r#"{"reasoning_budget":-1.5}"#, None)]
#[case::zero_budget(r#"{"reasoning_budget":0}"#, None)]
#[case::huge_budget(
    r#"{"reasoning_budget":18446744073709551615}"#,
    Some("18446744073709551615")
)]
#[case::bool_budget(r#"{"reasoning_budget":true}"#, None)]
#[case::low_effort(r#"{"low_effort":true}"#, Some(r#""low""#))]
#[case::low_effort_false(r#"{"low_effort":false}"#, None)]
#[case::enabled_without_budget(r#"{"enable_thinking":true}"#, None)]
#[case::unknown_keys_ignored(r#"{"enable_thinking":false,"foo":1}"#, Some(r#""none""#))]
#[tokio::test]
async fn chat_template_kwargs_forms_map(#[case] kwargs: &str, #[case] expected: Option<&str>) {
    let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
    let optional = Object::from_iter([(
        String::from("extra_body"),
        value(&format!(r#"{{"chat_template_kwargs":{kwargs}}}"#)),
    )]);
    let mapped = extra_body(&mut capabilities, "m", &optional).await;
    assert_eq!(mapped.get("reasoning_effort"), expected.map(value).as_ref());
}

#[tokio::test]
async fn explicit_reasoning_wins_over_chat_template_kwargs() {
    let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
    let in_optional = object(
        r#"{"reasoning_effort":"high","extra_body":{"chat_template_kwargs":{"enable_thinking":false}}}"#,
    );
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_optional).await,
        object(r#"{"reasoning_effort":"high"}"#)
    );
    let in_body = object(
        r#"{"extra_body":{"thinking":{"type":"enabled"},"chat_template_kwargs":{"enable_thinking":false}}}"#,
    );
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_body).await,
        object(r#"{"extra_body":{"thinking":{"type":"enabled"}}}"#)
    );
}

#[tokio::test]
async fn chat_template_kwargs_effort_needs_reasoning_support() {
    let mut capabilities = TestCapabilities::none();
    let optional =
        object(r#"{"extra_body":{"chat_template_kwargs":{"low_effort":true},"custom":1}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"extra_body":{"custom":1}}"#)
    );
}

#[tokio::test]
async fn a_non_map_chat_template_kwargs_is_dropped() {
    let mut capabilities = TestCapabilities::with(&[Gate::Reasoning]);
    let optional = object(r#"{"extra_body":{"chat_template_kwargs":"not-a-map","custom":1}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"extra_body":{"custom":1}}"#)
    );
    let null = object(r#"{"extra_body":{"chat_template_kwargs":null,"custom":1}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &null).await,
        object(r#"{"extra_body":{"custom":1}}"#)
    );
}

#[tokio::test]
async fn guided_json_becomes_a_named_json_schema() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"guided_json":{"type":"object"}}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(
            r#"{"response_format":{"type":"json_schema","json_schema":{"name":"response","schema":{"type":"object"}}}}"#
        )
    );
}

#[tokio::test]
async fn guided_grammar_becomes_a_grammar_response_format() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"guided_grammar":"start: \"yes\""}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(r#"{"response_format":{"type":"grammar","grammar":"start: \"yes\""}}"#)
    );
}

#[tokio::test]
async fn guided_choice_becomes_a_named_choice_schema() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"guided_choice":["yes","no"],"custom":1}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        object(
            r#"{"response_format":{"type":"json_schema","json_schema":{"name":"choice","schema":{"type":"string","enum":["yes","no"]}}},"extra_body":{"custom":1}}"#
        )
    );
}

#[tokio::test]
async fn guided_json_wins_over_the_other_guided_keys() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(
        r#"{"extra_body":{"guided_grammar":"start: \"x\"","guided_json":{"type":"object"}}}"#,
    );
    let mapped = extra_body(&mut capabilities, "m", &optional).await;
    assert_eq!(
        mapped.get("response_format"),
        Some(&value(
            r#"{"type":"json_schema","json_schema":{"name":"response","schema":{"type":"object"}}}"#
        ))
    );
    let choice_only = object(r#"{"extra_body":{"guided_json":null,"guided_choice":["one"]}}"#);
    let chosen = extra_body(&mut capabilities, "m", &choice_only).await;
    assert_eq!(
        chosen.get("response_format"),
        Some(&value(
            r#"{"type":"json_schema","json_schema":{"name":"choice","schema":{"type":"string","enum":["one"]}}}"#
        ))
    );
}

#[tokio::test]
async fn an_existing_response_format_wins_over_guided_params() {
    let mut capabilities = TestCapabilities::none();
    let in_optional = object(
        r#"{"response_format":{"type":"text"},"extra_body":{"guided_json":{"type":"object"}}}"#,
    );
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_optional).await,
        object(r#"{"response_format":{"type":"text"}}"#)
    );
    let in_body = object(
        r#"{"extra_body":{"response_format":{"type":"text"},"guided_json":{"type":"object"}}}"#,
    );
    assert_eq!(
        extra_body(&mut capabilities, "m", &in_body).await,
        object(r#"{"extra_body":{"response_format":{"type":"text"}}}"#)
    );
    let both = object(
        r#"{"response_format":{"type":"text"},"extra_body":{"response_format":{"type":"other"}}}"#,
    );
    assert_eq!(
        extra_body(&mut capabilities, "m", &both).await,
        object(r#"{"response_format":{"type":"text"}}"#)
    );
}

#[tokio::test]
async fn a_null_guided_key_stays_consumed() {
    let mut capabilities = TestCapabilities::none();
    let optional = object(r#"{"extra_body":{"guided_json":null}}"#);
    assert_eq!(
        extra_body(&mut capabilities, "m", &optional).await,
        Object::new()
    );
}

#[rstest]
#[case::json_schema_without_schema(r#"{"json_schema":{}}"#)]
#[case::json_schema_not_an_object(r#"{"json_schema":"text"}"#)]
#[case::response_format_not_an_object(r#""text""#)]
#[tokio::test]
async fn a_malformed_response_format_with_tools_is_a_bad_request(#[case] given: &str) {
    let mut capabilities = TestCapabilities::none();
    let mut optional = Object::new();
    let error = map_openai_params(
        &mut capabilities,
        &Catalog::new(),
        "m",
        &params(&[("tools", value("[]")), ("response_format", value(given))]),
        &mut optional,
    )
    .await
    .expect_err("a malformed response_format");
    assert_eq!(error.status_code(), Some(400));
    assert!(matches!(
        error,
        FireworksChatParamsError::ResponseFormat {
            source: ContextError::MissingSchema | ContextError::SchemaNotObject
        } | FireworksChatParamsError::ResponseFormatNotObject
    ));
}

#[tokio::test]
async fn a_malformed_tools_value_with_a_translated_format_is_a_bad_request() {
    let mut capabilities = TestCapabilities::none();
    let mut optional = object(r#"{"tools":"not-a-list"}"#);
    let error = map_openai_params(
        &mut capabilities,
        &Catalog::new(),
        "m",
        &params(&[
            ("tools", value("[]")),
            (
                "response_format",
                value(r#"{"response_schema":{"type":"object"}}"#),
            ),
        ]),
        &mut optional,
    )
    .await
    .expect_err("a malformed tools value");
    assert_eq!(error.status_code(), Some(400));
}

#[rstest]
#[case::null(r#"{"response_schema":null}"#, false)]
#[case::empty_string(r#"{"response_schema":""}"#, false)]
#[case::empty_list(r#"{"response_schema":[]}"#, false)]
#[case::empty_object(r#"{"response_schema":{}}"#, false)]
#[case::zero(r#"{"response_schema":0}"#, false)]
#[case::non_empty_string(r#"{"response_schema":"x"}"#, true)]
#[case::non_empty_list(r#"{"response_schema":[1]}"#, true)]
#[case::non_empty_object(r#"{"response_schema":{"type":"object"}}"#, true)]
#[case::number(r#"{"response_schema":5}"#, true)]
#[tokio::test]
async fn only_a_truthy_response_schema_becomes_a_tool(
    #[case] format: &str,
    #[case] translated: bool,
) {
    let mut capabilities = TestCapabilities::none();
    let mapped = map_request(
        &mut capabilities,
        "unknown-model",
        &[("tools", value("[]")), ("response_format", value(format))],
    )
    .await;
    assert_eq!(
        mapped.get("json_mode"),
        translated.then_some(Value::Bool(true)).as_ref()
    );
}

mod logs;
