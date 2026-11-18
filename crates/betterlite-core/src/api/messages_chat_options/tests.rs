#![cfg(test)]

use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::projections::reasoning_effort::ThinkingBudgetThresholds;
use betterlite_prompts::arguments::json::{Json, Object};

use super::context::OptionsContext;
use super::error::OptionsError;
use super::metadata::translate_metadata;
use super::passthrough::{copy_untranslated_params, translate_stop_sequences};
use super::system::{insert_system_message, mid_turn_system_message};
use super::thinking::{summary_wrapped_effort, thinking_effort, translate_thinking};
use crate::api::api_boundaries::ApiLimits;

const LIMITS: ApiLimits = ApiLimits {
    max_json_bytes: 4096,
    max_event_bytes: 4096,
    max_items: 64,
};

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("fixture literal is valid JSON")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("fixture literal is a JSON object")
}

struct Declared {
    names: Option<Vec<String>>,
    probes: Mutex<Vec<(String, Option<String>)>>,
}

impl Declared {
    fn new(names: Option<&[&str]>) -> Self {
        Self {
            names: names.map(|names| names.iter().map(|name| String::from(*name)).collect()),
            probes: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        let probes = self
            .probes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        probes.clone()
    }
}

impl SupportedParameterSource for Declared {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        let mut probes = self
            .probes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        probes.push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        Ok(self.names.clone())
    }
}

fn context<'request>(
    parameters: &'request Declared,
    model: &'request str,
    custom_llm_provider: Option<&'request str>,
) -> OptionsContext<'request> {
    OptionsContext {
        model,
        custom_llm_provider,
        auto_summary: false,
        thresholds: ThinkingBudgetThresholds::default(),
        limits: LIMITS,
        parameters,
    }
}

fn thinking_kwargs(
    request: &str,
    model: &str,
    custom_llm_provider: Option<&str>,
    declared: Option<&[&str]>,
    auto_summary: bool,
) -> (Object, usize) {
    let parameters = Declared::new(declared);
    let mut options = context(&parameters, model, custom_llm_provider);
    options.auto_summary = auto_summary;
    let mut kwargs = Object::new();
    translate_thinking(&object(request), &mut kwargs, &options).expect("the thinking translates");
    (kwargs, parameters.calls().len())
}

#[test]
fn thinking_budget_buckets() {
    let thresholds = ThinkingBudgetThresholds::default();
    let buckets = [
        ("null", None),
        (r#""adaptive""#, None),
        (r#"{"type": "bogus"}"#, None),
        (r#"{"type": 7}"#, None),
        ("{}", Some("none")),
        (r#"{"type": "disabled"}"#, Some("none")),
        (r#"{"type": null}"#, Some("none")),
        (r#"{"type": "adaptive"}"#, Some("medium")),
        (r#"{"type": "enabled"}"#, Some("minimal")),
        (
            r#"{"type": "enabled", "budget_tokens": null}"#,
            Some("minimal"),
        ),
        (
            r#"{"type": "enabled", "budget_tokens": 0}"#,
            Some("minimal"),
        ),
        (
            r#"{"type": "enabled", "budget_tokens": 1023}"#,
            Some("minimal"),
        ),
        (r#"{"type": "enabled", "budget_tokens": 1024}"#, Some("low")),
        (r#"{"type": "enabled", "budget_tokens": 2047}"#, Some("low")),
        (
            r#"{"type": "enabled", "budget_tokens": 2048}"#,
            Some("medium"),
        ),
        (
            r#"{"type": "enabled", "budget_tokens": 4095}"#,
            Some("medium"),
        ),
        (
            r#"{"type": "enabled", "budget_tokens": 4096}"#,
            Some("high"),
        ),
        (
            r#"{"type": "enabled", "budget_tokens": 8192}"#,
            Some("high"),
        ),
    ];
    for (input, expected) in buckets {
        assert_eq!(
            thinking_effort(&json(input), thresholds).expect("the bucket maps"),
            expected,
            "{input}"
        );
    }
}

#[test]
fn thinking_budgets_consume_changed_thresholds() {
    let changed = ThinkingBudgetThresholds {
        low: 10,
        medium: 20,
        high: 30,
    };
    let rows = [
        (r#"{"type": "enabled", "budget_tokens": 9}"#, "minimal"),
        (r#"{"type": "enabled", "budget_tokens": 10}"#, "low"),
        (r#"{"type": "enabled", "budget_tokens": 20}"#, "medium"),
        (r#"{"type": "enabled", "budget_tokens": 30}"#, "high"),
    ];
    for (input, expected) in rows {
        assert_eq!(
            thinking_effort(&json(input), changed).expect("the bucket maps"),
            Some(expected),
            "{input}"
        );
    }
}

#[test]
fn budget_tokens_of_another_type_is_an_error() {
    let thresholds = ThinkingBudgetThresholds::default();
    let rows = [
        r#"{"type": "enabled", "budget_tokens": "1024"}"#,
        r#"{"type": "enabled", "budget_tokens": 1024.5}"#,
        r#"{"type": "enabled", "budget_tokens": false}"#,
        r#"{"type": "enabled", "budget_tokens": [1024]}"#,
    ];
    for input in rows {
        assert!(
            matches!(
                thinking_effort(&json(input), thresholds),
                Err(OptionsError::BudgetNotInteger)
            ),
            "{input}"
        );
    }
}

#[test]
fn summary_wrapping() {
    let disabled = json(r#"{"type": "disabled", "summary": "detailed"}"#);
    assert_eq!(
        summary_wrapped_effort("none", &disabled, true),
        json(r#""none""#)
    );
    let concise = json(r#"{"type": "enabled", "budget_tokens": 1024, "summary": "concise"}"#);
    assert_eq!(
        summary_wrapped_effort("low", &concise, false),
        json(r#"{"effort": "low", "summary": "concise"}"#)
    );
    let empty = json(r#"{"type": "enabled", "summary": ""}"#);
    assert_eq!(
        summary_wrapped_effort("low", &empty, false),
        json(r#""low""#)
    );
    assert_eq!(
        summary_wrapped_effort("low", &empty, true),
        json(r#"{"effort": "low", "summary": "detailed"}"#)
    );
    let plain = json(r#"{"type": "enabled"}"#);
    assert_eq!(
        summary_wrapped_effort("low", &plain, false),
        json(r#""low""#)
    );
    assert_eq!(
        summary_wrapped_effort("low", &plain, true),
        json(r#"{"effort": "low", "summary": "detailed"}"#)
    );
    assert_eq!(
        summary_wrapped_effort("low", &json(r#""adaptive""#), true),
        json(r#"{"effort": "low", "summary": "detailed"}"#)
    );
}

#[test]
fn system_text_selection() {
    let model = Some("claude-sonnet-4-5");
    assert_eq!(
        mid_turn_system_message(&object(r#"{"role": "system", "content": "keep"}"#), model)
            .map(Json::Object),
        Some(json(r#"{"role": "system", "content": "keep"}"#))
    );
    for content in [
        r#""""#,
        r#"[]"#,
        r#"7"#,
        r#"null"#,
        r#"[{"type": "text", "text": ""}]"#,
        r#"[{"type": "text", "text": null}]"#,
        r#"[{"type": "image", "text": "not text"}]"#,
        r#"["block"]"#,
    ] {
        let message = object(&format!(r#"{{"role": "system", "content": {content}}}"#));
        assert_eq!(mid_turn_system_message(&message, model), None, "{content}");
    }
    let blocks = object(
        r#"{"role": "system", "content": [{"type": "text", "text": "one"}, {"type": "text", "text": ""}, {"type": "text", "text": "two"}]}"#,
    );
    assert_eq!(
        mid_turn_system_message(&blocks, model).map(Json::Object),
        Some(json(
            r#"{"role": "system", "content": [{"type": "text", "text": "one"}, {"type": "text", "text": "two"}]}"#
        ))
    );
}

#[test]
fn system_top_level_selection() {
    let block = object(
        r#"{"model": "m", "system": [{"type": "text"}, {"type": "image", "text": "x"}, {"type": "text", "text": "y"}]}"#,
    );
    let mut listed = Vec::new();
    insert_system_message(&mut listed, &block);
    assert_eq!(
        Json::List(listed),
        json(
            r#"[{"role": "system", "content": [{"type": "text", "text": ""}, {"type": "text", "text": "y"}]}]"#
        )
    );
    for system in ["", "null", "[]", "0", "{}", r#"["block"]"#] {
        let request = object(&format!(r#"{{"model": "m", "system": {system}}}"#));
        let mut messages = vec![json(r#"{"role": "user", "content": "hi"}"#)];
        insert_system_message(&mut messages, &request);
        assert_eq!(
            Json::List(messages),
            json(r#"[{"role": "user", "content": "hi"}]"#),
            "{system}"
        );
    }
}

#[test]
fn system_insertion_does_not_merge() {
    let request = object(r#"{"model": "m", "system": "new"}"#);
    let mut messages = vec![json(r#"{"role": "system", "content": "old"}"#)];
    insert_system_message(&mut messages, &request);
    assert_eq!(
        Json::List(messages),
        json(r#"[{"role": "system", "content": "new"}, {"role": "system", "content": "old"}]"#)
    );
}

#[test]
fn system_mid_turn_cache_control_and_breakpoint() {
    let block = object(
        r#"{"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}, "prompt_cache_breakpoint": false}"#,
    );
    let message = object(&format!(r#"{{"role": "system", "content": [{block}]}}"#));
    assert_eq!(
        mid_turn_system_message(&message, Some("anthropic/claude-sonnet-4-5")).map(Json::Object),
        Some(json(
            r#"{"role": "system", "content": [{"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}, "prompt_cache_breakpoint": false}]}"#
        ))
    );
    assert_eq!(
        mid_turn_system_message(&message, Some("gpt-4o")).map(Json::Object),
        Some(json(
            r#"{"role": "system", "content": [{"type": "text", "text": "x", "prompt_cache_breakpoint": false}]}"#
        ))
    );
}

#[test]
fn system_top_level_cache_control_and_breakpoint() {
    let block = object(
        r#"{"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}, "prompt_cache_breakpoint": false}"#,
    );
    let claude_request = object(&format!(
        r#"{{"model": "claude-sonnet-4-5", "system": [{block}]}}"#
    ));
    let mut claude_messages = Vec::new();
    insert_system_message(&mut claude_messages, &claude_request);
    assert_eq!(
        Json::List(claude_messages),
        json(
            r#"[{"role": "system", "content": [{"type": "text", "text": "x", "cache_control": {"type": "ephemeral"}, "prompt_cache_breakpoint": false}]}]"#
        )
    );
    let plain_request = object(&format!(r#"{{"model": "gpt-4o", "system": [{block}]}}"#));
    let mut plain_messages = Vec::new();
    insert_system_message(&mut plain_messages, &plain_request);
    assert_eq!(
        Json::List(plain_messages),
        json(
            r#"[{"role": "system", "content": [{"type": "text", "text": "x", "prompt_cache_breakpoint": false}]}]"#
        )
    );
}

#[test]
fn metadata_user_and_session() {
    let parameters = Declared::new(Some(&["prompt_cache_key"]));
    let options = context(&parameters, "claude-3", Some("anthropic"));
    for user in [r#""alice""#, r#""""#, "null"] {
        let mut request = object(&format!(r#"{{"metadata": {{"user_id": {user}}}}}"#));
        let mut kwargs = Object::new();
        translate_metadata(&mut request, &mut kwargs, &options).expect("metadata translates");
        assert_eq!(kwargs, object(&format!(r#"{{"user": {user}}}"#)), "{user}");
        assert!(parameters.calls().is_empty());
    }
    for request in [r#"{}"#, r#"{"metadata": null}"#, r#"{"metadata": {}}"#] {
        let mut absent = object(request);
        let mut kwargs = Object::new();
        translate_metadata(&mut absent, &mut kwargs, &options).expect("metadata translates");
        assert!(kwargs.is_empty(), "{request}");
    }
    assert!(parameters.calls().is_empty());
}

#[test]
fn metadata_session_cache_key() {
    let parameters = Declared::new(Some(&["prompt_cache_key"]));
    let options = context(&parameters, "claude-3", Some("anthropic"));
    let user_id = r#"{\"session_id\":\"session_a\"}"#;
    let mut request = object(&format!(r#"{{"metadata": {{"user_id": "{user_id}"}}}}"#));
    let mut kwargs = Object::new();
    translate_metadata(&mut request, &mut kwargs, &options).expect("metadata translates");
    assert_eq!(
        kwargs,
        object(&format!(
            r#"{{"user": "{user_id}", "prompt_cache_key": "session_a"}}"#
        ))
    );
    assert_eq!(
        parameters.calls(),
        vec![("claude-3".to_owned(), Some("anthropic".to_owned()))]
    );
}

#[test]
fn metadata_session_key_is_truncated_or_absent() {
    let parameters = Declared::new(Some(&["prompt_cache_key"]));
    let options = context(&parameters, "claude-3", Some("anthropic"));
    let empty = r#"{\"session_id\":\"\"}"#;
    let mut empty_request = object(&format!(r#"{{"metadata": {{"user_id": "{empty}"}}}}"#));
    let mut empty_kwargs = Object::new();
    translate_metadata(&mut empty_request, &mut empty_kwargs, &options)
        .expect("metadata translates");
    assert_eq!(empty_kwargs, object(&format!(r#"{{"user": "{empty}"}}"#)));
    assert!(parameters.calls().is_empty());
    let long = format!(r#"{{\"session_id\":\"{}\"}}"#, "a".repeat(70));
    let mut long_request = object(&format!(r#"{{"metadata": {{"user_id": "{long}"}}}}"#));
    let mut long_kwargs = Object::new();
    translate_metadata(&mut long_request, &mut long_kwargs, &options).expect("metadata translates");
    assert_eq!(
        long_kwargs["prompt_cache_key"],
        json(&format!(r#""{}""#, "a".repeat(64)))
    );
}

#[test]
fn metadata_cache_key_needs_a_declaring_provider() {
    let user_id = r#"{\"session_id\":\"session_a\"}"#;
    let rows = [
        (
            Some("litellm_proxy"),
            "claude-3",
            Some(&["prompt_cache_key"][..]),
            0,
        ),
        (None, "claude-3", Some(&["prompt_cache_key"][..]), 0),
        (Some("anthropic"), "", Some(&["prompt_cache_key"][..]), 0),
        (Some("anthropic"), "claude-3", None, 1),
        (
            Some("anthropic"),
            "claude-3",
            Some(&["reasoning_effort"][..]),
            1,
        ),
        (Some("anthropic"), "claude-3", Some(&[]), 1),
    ];
    for (provider, model, declared, expected_probes) in rows {
        let parameters = Declared::new(declared);
        let options = context(&parameters, model, provider);
        let mut request = object(&format!(r#"{{"metadata": {{"user_id": "{user_id}"}}}}"#));
        let mut kwargs = Object::new();
        translate_metadata(&mut request, &mut kwargs, &options).expect("metadata translates");
        assert!(
            !kwargs.contains_key("prompt_cache_key"),
            "{provider:?} {model:?} {declared:?}"
        );
        assert_eq!(
            parameters.calls().len(),
            expected_probes,
            "{provider:?} {model:?} {declared:?}"
        );
    }
}

#[test]
fn metadata_litellm_metadata_is_popped() {
    let parameters = Declared::new(None);
    let options = context(&parameters, "claude-3", None);
    let mut request = object(r#"{"metadata": {"user_id": "alice"}, "litellm_metadata": {"x": 1}}"#);
    let mut kwargs = Object::new();
    translate_metadata(&mut request, &mut kwargs, &options).expect("metadata translates");
    assert_eq!(kwargs, object(r#"{"user": "alice", "metadata": {"x": 1}}"#));
    assert_eq!(request, object(r#"{"metadata": {"user_id": "alice"}}"#));
}

#[test]
fn metadata_null_litellm_metadata_is_popped() {
    let parameters = Declared::new(None);
    let options = context(&parameters, "claude-3", None);
    let mut request = object(r#"{"litellm_metadata": null}"#);
    let mut kwargs = Object::new();
    translate_metadata(&mut request, &mut kwargs, &options).expect("metadata translates");
    assert_eq!(kwargs, object(r#"{"metadata": null}"#));
    assert!(request.is_empty());
}

#[test]
fn metadata_session_key_respects_the_request_limit() {
    let parameters = Declared::new(Some(&["prompt_cache_key"]));
    let mut options = context(&parameters, "claude-3", Some("anthropic"));
    options.limits = ApiLimits {
        max_json_bytes: 8,
        max_event_bytes: 8,
        max_items: 8,
    };
    let mut request = object(r#"{"metadata": {"user_id": "{\"session_id\":\"session_a\"}"}}"#);
    let mut kwargs = Object::new();
    assert!(matches!(
        translate_metadata(&mut request, &mut kwargs, &options),
        Err(OptionsError::SessionKey(_))
    ));
}

#[test]
fn claude_thinking_carriers() {
    let thinking = r#"{"type": "adaptive", "display": "updates"}"#;
    let request = format!(r#"{{"thinking": {thinking}}}"#);
    let (kwargs, probes) = thinking_kwargs(
        &request,
        "anthropic/claude-sonnet-4-5",
        Some("anthropic"),
        Some(&["thinking"]),
        false,
    );
    assert_eq!(kwargs, object(&request));
    assert_eq!(probes, 1);
}

#[test]
fn claude_thinking_carriers_forward_bedrock_config() {
    let thinking = r#"{"type": "adaptive", "display": "updates"}"#;
    let config = r#"{"effort": "high", "format": {}, "other": 7}"#;
    let request = format!(r#"{{"thinking": {thinking}, "output_config": {config}}}"#);
    let expected = object(&format!(
        r#"{{"thinking": {thinking}, "output_config": {{"effort": "high", "other": 7}}}}"#
    ));
    for model in [
        "bedrock/anthropic.claude-sonnet-4-5",
        "converse/anthropic.claude-sonnet-4-5",
        "invoke/anthropic.claude-sonnet-4-5",
        "arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/abc",
    ] {
        let (kwargs, probes) = thinking_kwargs(
            &request,
            model,
            Some("bedrock"),
            Some(&["reasoning_effort"]),
            false,
        );
        assert_eq!(kwargs, expected, "{model}");
        assert_eq!(probes, 0, "{model}");
    }
}

#[test]
fn claude_thinking_carriers_carry_the_declared_effort() {
    let thinking = r#"{"type": "adaptive", "display": "updates"}"#;
    let request = format!(r#"{{"thinking": {thinking}, "output_config": {{"effort": "high"}}}}"#);
    let (kwargs, probes) = thinking_kwargs(
        &request,
        "vertex_ai/claude-sonnet-4-5",
        Some("vertex_ai"),
        Some(&["reasoning_effort"]),
        false,
    );
    assert_eq!(
        kwargs,
        object(&format!(
            r#"{{"thinking": {thinking}, "reasoning_effort": "high"}}"#
        ))
    );
    assert_eq!(probes, 1);
    let without = format!(r#"{{"thinking": {thinking}}}"#);
    let (plain, _) = thinking_kwargs(
        &without,
        "vertex_ai/claude-sonnet-4-5",
        Some("vertex_ai"),
        Some(&["reasoning_effort"]),
        false,
    );
    assert_eq!(plain, object(&without));
}

#[test]
fn claude_thinking_carriers_need_a_declaring_provider() {
    let thinking = r#"{"type": "adaptive", "display": "updates"}"#;
    let config = r#"{"effort": "high"}"#;
    let request = format!(r#"{{"thinking": {thinking}, "output_config": {config}}}"#);
    let expected = object(&format!(r#"{{"thinking": {thinking}}}"#));
    let (quiet, probes) = thinking_kwargs(
        &request,
        "vertex_ai/claude-sonnet-4-5",
        Some("vertex_ai"),
        Some(&["thinking"]),
        false,
    );
    assert_eq!(quiet, expected);
    assert_eq!(probes, 1);
    let (absent, no_probes) = thinking_kwargs(&request, "claude-sonnet-4-5", None, None, false);
    assert_eq!(absent, expected);
    assert_eq!(no_probes, 0);
    let (proxied, proxy_probes) = thinking_kwargs(
        &request,
        "claude-sonnet-4-5",
        Some("litellm_proxy"),
        Some(&["reasoning_effort"]),
        false,
    );
    assert_eq!(
        proxied,
        object(&format!(
            r#"{{"thinking": {thinking}, "reasoning_effort": "high"}}"#
        ))
    );
    assert_eq!(proxy_probes, 1);
}

#[test]
fn claude_thinking_carriers_skip_non_adaptive_effort() {
    let request = r#"{"thinking": {"type": "enabled", "budget_tokens": 9000}, "output_config": {"effort": "high"}}"#;
    let expected = object(r#"{"thinking": {"type": "enabled", "budget_tokens": 9000}}"#);
    let (kwargs, _) = thinking_kwargs(
        request,
        "vertex_ai/claude-sonnet-4-5",
        Some("vertex_ai"),
        Some(&["reasoning_effort"]),
        false,
    );
    assert_eq!(kwargs, expected);
    let (bedrock, _) = thinking_kwargs(
        request,
        "bedrock/anthropic.claude-sonnet-4-5",
        Some("bedrock"),
        None,
        false,
    );
    assert_eq!(
        bedrock,
        object(r#"{"thinking": {"type": "enabled", "budget_tokens": 9000}}"#)
    );
}

#[test]
fn claude_thinking_carriers_forward_only_effort_fields() {
    for config in [r#"{"format": {}}"#, r#""high""#, "null"] {
        let request =
            format!(r#"{{"thinking": {{"type": "adaptive"}}, "output_config": {config}}}"#);
        let (kwargs, _) = thinking_kwargs(
            &request,
            "bedrock/anthropic.claude-sonnet-4-5",
            Some("bedrock"),
            None,
            false,
        );
        assert_eq!(
            kwargs,
            object(r#"{"thinking": {"type": "adaptive"}}"#),
            "{config}"
        );
    }
}

#[test]
fn nonclaude_thinking_carriers() {
    let request = r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}}"#;
    let (kwargs, probes) = thinking_kwargs(request, "gpt-4o", Some("openai"), None, false);
    assert_eq!(kwargs, object(r#"{"reasoning_effort": "high"}"#));
    assert_eq!(probes, 0);
    let enabled = r#"{"thinking": {"type": "enabled", "budget_tokens": 1023}, "output_config": {"effort": "high"}}"#;
    let (bucketed, _) = thinking_kwargs(enabled, "gpt-4o", Some("openai"), None, false);
    assert_eq!(bucketed, object(r#"{"reasoning_effort": "minimal"}"#));
    let disabled = r#"{"thinking": {"type": "disabled", "summary": "detailed"}}"#;
    let (none, _) = thinking_kwargs(disabled, "gpt-4o", Some("openai"), None, true);
    assert_eq!(none, object(r#"{"reasoning_effort": "none"}"#));
    let unknown = r#"{"thinking": {"type": "bogus"}, "output_config": {"effort": "high"}}"#;
    let (skipped, _) = thinking_kwargs(unknown, "gpt-4o", Some("openai"), None, false);
    assert!(skipped.is_empty());
}

#[test]
fn nonclaude_thinking_carriers_wrap_the_summary() {
    let adaptive = r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}}"#;
    let (detailed, _) = thinking_kwargs(adaptive, "gpt-4o", Some("openai"), None, true);
    assert_eq!(
        detailed,
        object(r#"{"reasoning_effort": {"effort": "high", "summary": "detailed"}}"#)
    );
    let summary = r#"{"thinking": {"type": "adaptive", "summary": "concise"}, "output_config": {"effort": "high"}}"#;
    let (concise, _) = thinking_kwargs(summary, "gpt-4o", Some("openai"), None, true);
    assert_eq!(
        concise,
        object(r#"{"reasoning_effort": {"effort": "high", "summary": "concise"}}"#)
    );
}

#[test]
fn nonclaude_thinking_carriers_skip_falsey_thinking() {
    for thinking in ["null", r#""""#, "{}", "[]", "0", "false"] {
        let request = format!(r#"{{"thinking": {thinking}}}"#);
        let (kwargs, _) = thinking_kwargs(&request, "gpt-4o", Some("openai"), None, true);
        assert!(kwargs.is_empty(), "{thinking}");
    }
}

#[test]
fn thinking_effort_of_another_type_is_an_error() {
    let parameters = Declared::new(None);
    let options = context(&parameters, "gpt-4o", Some("openai"));
    let request = r#"{"thinking": {"type": "adaptive"}, "output_config": {"effort": 7}}"#;
    let mut kwargs = Object::new();
    assert!(matches!(
        translate_thinking(&object(request), &mut kwargs, &options),
        Err(OptionsError::EffortNotText)
    ));
    let enabled = r#"{"thinking": {"type": "enabled", "budget_tokens": 1024}, "output_config": {"effort": 7}}"#;
    let mut bucketed = Object::new();
    translate_thinking(&object(enabled), &mut bucketed, &options).expect("the effort is ignored");
    assert_eq!(bucketed, object(r#"{"reasoning_effort": "low"}"#));
}

#[test]
fn passthrough_stop_sequences() {
    let mut kwargs = Object::new();
    translate_stop_sequences(&object(r#"{"stop_sequences": ["END"]}"#), &mut kwargs);
    assert_eq!(kwargs, object(r#"{"stop": ["END"]}"#));
    for request in [
        r#"{"stop_sequences": []}"#,
        r#"{"stop_sequences": null}"#,
        r#"{"stop_sequences": ""}"#,
        "{}",
    ] {
        let mut empty = Object::new();
        translate_stop_sequences(&object(request), &mut empty);
        assert!(empty.is_empty(), "{request}");
    }
}

#[test]
fn passthrough_copy_excludes_translated_params() {
    let request = object(
        r#"{"messages": [], "metadata": {}, "system": "s", "tool_choice": null, "tools": [], "thinking": {}, "output_format": null, "output_config": null, "stop_sequences": ["END"], "max_tokens": 0, "stream": false, "temperature": null, "unknown": 7, "model": "m", "stop": ["x"], "reasoning_effort": "low"}"#,
    );
    let mut kwargs = Object::new();
    copy_untranslated_params(&request, &mut kwargs);
    assert_eq!(
        kwargs,
        object(
            r#"{"max_tokens": 0, "stream": false, "temperature": null, "unknown": 7, "model": "m", "stop": ["x"], "reasoning_effort": "low"}"#
        )
    );
}

#[test]
fn passthrough_copy_overwrites_translated_values() {
    let request = object(
        r#"{"thinking": {"type": "disabled"}, "stop_sequences": ["END"], "reasoning_effort": "low", "stop": ["OVERRIDE"]}"#,
    );
    let parameters = Declared::new(None);
    let options = context(&parameters, "gpt-4o", Some("openai"));
    let mut kwargs = Object::new();
    translate_thinking(&request, &mut kwargs, &options).expect("the thinking translates");
    translate_stop_sequences(&request, &mut kwargs);
    assert_eq!(
        kwargs,
        object(r#"{"reasoning_effort": "none", "stop": ["END"]}"#)
    );
    copy_untranslated_params(&request, &mut kwargs);
    assert_eq!(
        kwargs,
        object(r#"{"reasoning_effort": "low", "stop": ["OVERRIDE"]}"#)
    );
}
