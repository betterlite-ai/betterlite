use std::collections::BTreeMap;

use crate::arguments::json::Json;
use crate::cache_placement::chat::{ChatPrompt, apply_chat_injections};
use crate::cache_placement::error::PlacementError;
use crate::cache_placement::tests::support::{Facts, json, messages, object, wire};

struct World<'input> {
    environment: BTreeMap<String, String>,
    global_api_base: Option<&'input str>,
    facts: Facts,
}

impl World<'_> {
    fn plain() -> Self {
        Self {
            environment: BTreeMap::new(),
            global_api_base: None,
            facts: Facts::none(),
        }
    }
}

fn run(messages_wire: &str, model: &str, params_wire: &str, world: &World<'_>) -> (Json, Json) {
    let conversation = messages(messages_wire);
    let mut params = object(params_wire);
    let prompt = ChatPrompt {
        model,
        messages: &conversation,
        params: &mut params,
        environment: &world.environment,
        global_api_base: world.global_api_base,
        facts: &world.facts,
    };
    let processed = apply_chat_injections(prompt).unwrap();
    (wire(&processed), wire(&params))
}

const USER: &str = r#"[{"location": "message", "role": "user", "index": null, "control": null}]"#;

fn stamped(dialect: bool) -> String {
    format!(
        r#"[{{"location": "message", "role": "user", "index": null, "control": null, "_litellm_openai_dialect": {dialect}}}]"#
    )
}

#[test]
fn a_missing_points_key_returns_the_messages_unchanged() {
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        "{}",
        &World::plain(),
    );
    assert_eq!(processed, json(r#"[{"role": "user", "content": "hi"}]"#));
    assert_eq!(params, json("{}"));
}

#[test]
fn an_empty_point_list_is_removed_from_the_params() {
    let params_wire = r#"{"temperature": 0.5, "cache_control_injection_points": []}"#;
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        params_wire,
        &World::plain(),
    );
    assert_eq!(processed, json(r#"[{"role": "user", "content": "hi"}]"#));
    assert_eq!(params, json(r#"{"temperature": 0.5}"#));
}

#[test]
fn a_null_point_value_is_removed_from_the_params() {
    let params_wire = r#"{"cache_control_injection_points": null}"#;
    let (_, params) = run("[]", "m", params_wire, &World::plain());
    assert_eq!(params, json("{}"));
}

#[test]
fn the_carry_flag_is_removed_even_without_points() {
    let params_wire = r#"{"_litellm_carry_unmatched_cache_control_points": true, "cache_control_injection_points": []}"#;
    let (_, params) = run("[]", "m", params_wire, &World::plain());
    assert_eq!(params, json("{}"));
}

#[test]
fn a_stamped_anthropic_point_marks_string_content_at_the_top_level() {
    let params_wire = format!(
        r#"{{"cache_control_injection_points": {}}}"#,
        stamped(false)
    );
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert_eq!(params, json("{}"));
}

#[test]
fn a_stamped_openai_point_marks_and_sets_the_explicit_mode() {
    let params_wire = format!(r#"{{"cache_control_injection_points": {}}}"#, stamped(true));
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#
        )
    );
    assert_eq!(
        params,
        json(r#"{"prompt_cache_options": {"mode": "explicit"}}"#)
    );
}

#[test]
fn an_existing_prompt_cache_options_is_not_replaced() {
    let params_wire = format!(
        r#"{{"cache_control_injection_points": {}, "prompt_cache_options": {{"mode": "implicit"}}}}"#,
        stamped(true)
    );
    let (_, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        params,
        json(r#"{"prompt_cache_options": {"mode": "implicit"}}"#)
    );
}

#[test]
fn openai_without_an_added_mark_sets_no_explicit_mode() {
    let params_wire = r#"{"cache_control_injection_points": [{"location": "message", "role": "assistant", "index": null, "control": null, "_litellm_openai_dialect": true}]}"#;
    let (processed, params) = run(
        r#"[{"role": "assistant", "content": "hi"}]"#,
        "m",
        params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "assistant", "content": "hi"}]"#)
    );
    assert_eq!(params, json("{}"));
}

#[test]
fn the_carry_flag_defers_a_role_point_without_an_index() {
    let params_wire = format!(
        r#"{{"_litellm_carry_unmatched_cache_control_points": true, "cache_control_injection_points": {USER}}}"#
    );
    let expected_params = format!(r#"{{"cache_control_injection_points": {USER}}}"#);
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(processed, json(r#"[{"role": "user", "content": "hi"}]"#));
    assert_eq!(params, json(&expected_params));
}

#[test]
fn the_carry_flag_defers_a_role_point_even_without_its_role() {
    let params_wire = format!(
        r#"{{"_litellm_carry_unmatched_cache_control_points": true, "cache_control_injection_points": {USER}}}"#
    );
    let expected_params = format!(r#"{{"cache_control_injection_points": {USER}}}"#);
    let (processed, params) = run(
        r#"[{"role": "assistant", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "assistant", "content": "hi"}]"#)
    );
    assert_eq!(params, json(&expected_params));
}

#[test]
fn the_carry_flag_applies_an_indexed_point() {
    let points = r#"[{"location": "message", "role": null, "index": 1, "control": null}, {"location": "message", "role": "user", "index": null, "control": null}]"#;
    let params_wire = format!(
        r#"{{"_litellm_carry_unmatched_cache_control_points": true, "cache_control_injection_points": {points}}}"#
    );
    let input = r#"[{"role": "user", "content": "a"}, {"role": "user", "content": "b"}]"#;
    let (processed, params) = run(input, "m", &params_wire, &World::plain());
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": "a"}, {"role": "user", "content": "b", "cache_control": {"type": "ephemeral"}}]"#
        )
    );
    let expected = json(
        r#"{"cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#,
    );
    assert_eq!(params, expected);
}

#[test]
fn without_the_carry_flag_a_role_point_is_applied() {
    let params_wire = format!(r#"{{"cache_control_injection_points": {USER}}}"#);
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert_eq!(params, json("{}"));
}

#[test]
fn a_false_carry_flag_applies_the_role_point() {
    let params_wire = format!(
        r#"{{"_litellm_carry_unmatched_cache_control_points": false, "cache_control_injection_points": {USER}}}"#
    );
    let (processed, _) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "m",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
}

#[test]
fn a_tool_config_point_is_forwarded() {
    let params_wire =
        r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#;
    let (_, params) = run("[]", "m", params_wire, &World::plain());
    assert_eq!(
        params,
        json(
            r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#
        )
    );
}

#[test]
fn a_tool_config_point_is_dropped_when_the_wire_is_full() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d", "cache_control": {}}
    ]"#;
    let params_wire =
        r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#;
    let (_, params) = run(input, "m", params_wire, &World::plain());
    assert_eq!(params, json("{}"));
}

#[test]
fn openai_forwards_a_tool_config_point_even_when_full() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "user", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d", "cache_control": {}}
    ]"#;
    let params_wire = r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null, "_litellm_openai_dialect": true}]}"#;
    let (_, params) = run(input, "m", params_wire, &World::plain());
    assert_eq!(
        params,
        json(
            r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null, "_litellm_openai_dialect": true}]}"#
        )
    );
}

#[test]
fn a_carried_point_keeps_both_stamps() {
    let params_wire = r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null, "_litellm_openai_dialect": false, "_litellm_external_breakpoints": 2}]}"#;
    let (_, params) = run("[]", "m", params_wire, &World::plain());
    assert_eq!(
        params,
        json(
            r#"{"cache_control_injection_points": [{"location": "tool_config", "control": null, "_litellm_openai_dialect": false, "_litellm_external_breakpoints": 2}]}"#
        )
    );
}

#[test]
fn an_external_breakpoint_stamp_shrinks_the_budget() {
    let input = r#"[{"role": "user", "content": "a", "cache_control": {}}, {"role": "user", "content": "b"}]"#;
    let points = r#"[{"location": "message", "role": "user", "index": null, "control": null, "_litellm_external_breakpoints": 3}]"#;
    let params_wire = format!(r#"{{"cache_control_injection_points": {points}}}"#);
    let (processed, params) = run(input, "m", &params_wire, &World::plain());
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": "a", "cache_control": {}}, {"role": "user", "content": "b"}]"#
        )
    );
    assert_eq!(params, json("{}"));
}

#[test]
fn the_dialect_falls_back_to_the_model_and_provider() {
    let params_wire = r#"{"custom_llm_provider": "openai", "api_base": "https://api.openai.com/v1", "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#
        )
    );
    assert_eq!(
        params,
        json(
            r#"{"custom_llm_provider": "openai", "api_base": "https://api.openai.com/v1", "prompt_cache_options": {"mode": "explicit"}}"#
        )
    );
}

#[test]
fn the_environment_base_url_selects_the_openai_dialect() {
    let world = World {
        environment: BTreeMap::from([(
            "OPENAI_BASE_URL".to_owned(),
            "https://api.openai.com/v1".to_owned(),
        )]),
        global_api_base: None,
        facts: Facts::none(),
    };
    let params_wire = r#"{"custom_llm_provider": "openai", "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let (processed, _) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        params_wire,
        &world,
    );
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#
        )
    );
}

#[test]
fn another_provider_stays_on_the_anthropic_dialect() {
    let params_wire = r#"{"custom_llm_provider": "azure", "api_base": "https://api.openai.com/v1", "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let (processed, _) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
}

#[test]
fn a_non_openai_base_url_stays_on_the_anthropic_dialect() {
    let params_wire = r#"{"custom_llm_provider": "openai", "api_base": "https://example.test/v1", "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let (processed, _) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
}

#[test]
fn a_present_prompt_cache_options_selects_the_openai_dialect() {
    let params_wire = r#"{"custom_llm_provider": "openai", "api_base": "https://example.test/v1", "prompt_cache_options": {}, "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}]}"#;
    let (processed, _) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "prompt_cache_breakpoint": {"mode": "explicit"}}]}]"#
        )
    );
}

#[test]
fn the_first_point_decides_the_dialect() {
    let points = r#"[{"location": "tool_config", "control": null}, {"location": "message", "role": "user", "index": null, "control": null, "_litellm_openai_dialect": true}]"#;
    let params_wire = format!(
        r#"{{"custom_llm_provider": "azure", "cache_control_injection_points": {points}}}"#
    );
    let (processed, params) = run(
        r#"[{"role": "user", "content": "hi"}]"#,
        "gpt-5.6",
        &params_wire,
        &World::plain(),
    );
    assert_eq!(
        processed,
        json(r#"[{"role": "user", "content": "hi", "cache_control": {"type": "ephemeral"}}]"#)
    );
    assert_eq!(
        params,
        json(
            r#"{"custom_llm_provider": "azure", "cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#
        )
    );
}

#[test]
fn malformed_points_are_a_typed_error() {
    let conversation = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let mut params = object(r#"{"cache_control_injection_points": "nope"}"#);
    let world = World::plain();
    let prompt = ChatPrompt {
        model: "m",
        messages: &conversation,
        params: &mut params,
        environment: &world.environment,
        global_api_base: None,
        facts: &world.facts,
    };
    let error = apply_chat_injections(prompt).unwrap_err();
    assert!(matches!(error, PlacementError::InjectionPoints { .. }));
}

#[test]
fn a_tool_config_point_reserves_a_slot_from_message_injection() {
    let input = r#"[
        {"role": "user", "content": "a", "cache_control": {}},
        {"role": "assistant", "content": "b", "cache_control": {}},
        {"role": "user", "content": "c", "cache_control": {}},
        {"role": "user", "content": "d"}
    ]"#;
    let params_wire = r#"{"custom_llm_provider": "azure", "cache_control_injection_points": [{"location": "message", "role": "user", "index": null, "control": null}, {"location": "tool_config", "control": null}]}"#;
    let (processed, params) = run(input, "gpt-5.6", params_wire, &World::plain());
    assert_eq!(processed, json(input));
    let expected = json(
        r#"{"custom_llm_provider": "azure", "cache_control_injection_points": [{"location": "tool_config", "control": null}]}"#,
    );
    assert_eq!(params, expected);
}
