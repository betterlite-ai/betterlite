#![cfg(test)]

use std::collections::VecDeque;
use std::mem::take;
use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::error::CountError;
use rstest::rstest;

use super::blocks::{cleared_tool_result_content, tool_ids};
use super::constants::{
    CLEAR_TOOL_USES_EDIT_TYPE, CLEARED_TOOL_RESULT_CONTENT, COMPACT_DEFAULT_INSTRUCTIONS,
    COMPACT_DEFAULT_TRIGGER_TOKENS, COMPACT_EDIT_TYPE, COMPACT_MIN_TRIGGER_TOKENS,
    COMPACT_NO_TOOL_CALLS_SUFFIX, COMPACT_SUMMARY_MAX_TOKENS, COMPACT_SUMMARY_SYSTEM_PREFIX,
    DEFAULT_INPUT_TOKENS_TRIGGER, DEFAULT_KEEP_TOOL_USES, SUMMARY_MAX_TOKENS_KEY,
    SUMMARY_MODEL_KEY, SUMMARY_TIMEOUT_SECONDS,
};
use super::counter::{TokenCountRequest, TokenCounter};
use super::editor::{ClearToolUsesOutcome, EditorRequest, apply_clear_tool_uses_20250919};
use super::error::{ClearToolUsesError, ContextManagementError};
use super::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};
use super::spec::ClearToolUsesEditSpec;

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("valid fixture")
}

fn content(message: &Json) -> &Json {
    match message {
        Json::Object(fields) => fields.get("content").expect("content"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("message object")
        }
    }
}

fn spec(source: &str) -> ClearToolUsesEditSpec {
    serde_json::from_str(source).expect("valid edit spec")
}

fn conversation() -> Vec<Json> {
    vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"lookup","input":{"query":"one"}},{"type":"tool_use","id":"b","name":"lookup","input":{"query":"two"}},{"type":"tool_use","id":"c","name":"lookup","input":{"query":"three"}},{"type":"tool_use","id":"d","name":"lookup","input":{"query":"four"}}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"result a"},{"type":"tool_result","tool_use_id":"b","content":"result b"},{"type":"tool_result","tool_use_id":"c","content":"result c"},{"type":"tool_result","tool_use_id":"d","content":"result d"}]}"#,
        ),
    ]
}

fn generated_conversation(ids: &[&str], results: &[&str]) -> Vec<Json> {
    let uses: Vec<String> = ids
        .iter()
        .map(|id| format!(r#"{{"type":"tool_use","id":"{id}"}}"#))
        .collect();
    let answered: Vec<String> = results
        .iter()
        .map(|id| {
            format!(r#"{{"type":"tool_result","tool_use_id":"{id}","content":"result {id}"}}"#)
        })
        .collect();
    vec![
        json(&format!(
            r#"{{"role":"assistant","content":[{}]}}"#,
            uses.join(",")
        )),
        json(&format!(
            r#"{{"role":"user","content":[{}]}}"#,
            answered.join(",")
        )),
    ]
}

#[derive(Debug)]
struct RecordedCall {
    model: String,
    messages: Vec<Json>,
    tools: Option<Vec<Json>>,
    count: usize,
}

struct ScriptedCounter {
    values: Mutex<VecDeque<usize>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedCounter {
    fn new(values: &[usize]) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        let mut calls = self.calls.lock().expect("call lock");
        take(&mut calls)
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("call lock").len()
    }
}

impl TokenCounter for ScriptedCounter {
    fn count(&self, request: TokenCountRequest<'_>) -> Result<usize, ClearToolUsesError> {
        let value = {
            let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
            values
                .pop_front()
                .ok_or(ClearToolUsesError::TokenCount(CountError::Overflow))?
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                model: request.model.to_owned(),
                messages: request.messages.to_vec(),
                tools: request.tools.map(<[Json]>::to_vec),
                count: value,
            });
        Ok(value)
    }
}

fn run_editor(
    messages: Vec<Json>,
    spec_source: &str,
    counter: &dyn TokenCounter,
) -> ClearToolUsesOutcome {
    let edit_spec = spec(spec_source);
    apply_clear_tool_uses_20250919(
        EditorRequest {
            model: "gpt-4o-mini",
            messages,
            tools: None,
            system: None,
            edit_spec: &edit_spec,
        },
        counter,
    )
    .expect("editor succeeds")
}

#[test]
fn default_input_token_threshold_does_not_trigger_at_the_limit() {
    let counter = ScriptedCounter::new(&[100_000]);
    let outcome = run_editor(conversation(), "{}", &counter);
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[test]
fn default_input_token_threshold_triggers_above_the_limit() {
    let counter = ScriptedCounter::new(&[100_001, 80]);
    let outcome = run_editor(conversation(), "{}", &counter);
    let applied = outcome.applied.expect("edit");
    assert_eq!(applied.cleared_tool_uses, Some(1));
    assert_eq!(applied.cleared_input_tokens, Some(99_921));
    assert_eq!(counter.call_count(), 2);
}

#[test]
fn empty_trigger_and_keep_objects_take_the_defaults() {
    let counter = ScriptedCounter::new(&[100_000]);
    let outcome = run_editor(conversation(), r#"{"trigger":{},"keep":{}}"#, &counter);
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[test]
fn unknown_trigger_type_counts_input_tokens() {
    let counter = ScriptedCounter::new(&[100_001, 80]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"mystery"}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
    assert_eq!(counter.call_count(), 2);
}

#[rstest]
#[case::string(r#""many""#)]
#[case::float("3.5")]
#[case::null("null")]
fn noninteger_trigger_value_uses_the_default_threshold(#[case] value: &str) {
    let counter = ScriptedCounter::new(&[100_000]);
    let spec_source = format!(r#"{{"trigger":{{"value":{value}}}}}"#);
    let outcome = run_editor(conversation(), &spec_source, &counter);
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[rstest]
#[case::string(r#""many""#)]
#[case::float("3.5")]
#[case::null("null")]
fn noninteger_trigger_value_triggers_above_the_default_threshold(#[case] value: &str) {
    let counter = ScriptedCounter::new(&[100_001, 80]);
    let spec_source = format!(r#"{{"trigger":{{"value":{value}}}}}"#);
    let outcome = run_editor(conversation(), &spec_source, &counter);
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
    assert_eq!(counter.call_count(), 2);
}

#[test]
fn a_negative_input_threshold_is_always_met() {
    let counter = ScriptedCounter::new(&[0, 0]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":-1}}"#, &counter);
    let applied = outcome.applied.expect("edit");
    assert_eq!(applied.cleared_tool_uses, Some(1));
    assert_eq!(applied.cleared_input_tokens, Some(0));
}

#[test]
fn a_threshold_beyond_machine_words_is_not_met() {
    let counter = ScriptedCounter::new(&[0]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"value":1000000000000000000000000000000}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[test]
fn a_threshold_above_the_integer_range_is_not_met() {
    let counter = ScriptedCounter::new(&[0]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"value":10000000000000000000000000000000000000000}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[test]
fn a_threshold_below_the_integer_range_is_met() {
    let counter = ScriptedCounter::new(&[0, 0]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"value":-10000000000000000000000000000000000000000}}"#,
        &counter,
    );
    let applied = outcome.applied.expect("edit");
    assert_eq!(applied.cleared_tool_uses, Some(1));
    assert_eq!(applied.cleared_input_tokens, Some(0));
}

#[test]
fn a_keep_value_above_the_integer_range_keeps_everything() {
    let counter = ScriptedCounter::new(&[]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":10000000000000000000000000000000000000000}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 0);
}

#[test]
fn a_keep_value_below_the_integer_range_uses_the_default() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":-10000000000000000000000000000000000000000}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
}

#[test]
fn tool_use_threshold_triggers_above_it() {
    let counter = ScriptedCounter::new(&[100, 40]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":3}}"#,
        &counter,
    );
    let applied = outcome.applied.expect("edit");
    assert_eq!(applied.cleared_tool_uses, Some(1));
    assert_eq!(applied.cleared_input_tokens, Some(60));
    assert_eq!(counter.call_count(), 2);
}

#[test]
fn tool_use_threshold_at_the_count_does_not_trigger() {
    let counter = ScriptedCounter::new(&[]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":4}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 0);
}

#[test]
fn noninteger_tool_threshold_does_not_count_tokens() {
    let counter = ScriptedCounter::new(&[]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":3.5}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 0);
}

#[test]
fn boolean_true_input_threshold_is_one_and_not_triggered_at_one() {
    let counter = ScriptedCounter::new(&[1]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":true}}"#, &counter);
    assert!(outcome.applied.is_none());
}

#[test]
fn boolean_true_input_threshold_is_one_and_triggers_above_one() {
    let counter = ScriptedCounter::new(&[2, 0]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":true}}"#, &counter);
    assert!(outcome.applied.is_some());
}

#[test]
fn boolean_false_input_threshold_is_zero_and_not_triggered_at_zero() {
    let counter = ScriptedCounter::new(&[0]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":false}}"#, &counter);
    assert!(outcome.applied.is_none());
}

#[test]
fn boolean_false_input_threshold_is_zero_and_triggers_above_zero() {
    let counter = ScriptedCounter::new(&[1, 0]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":false}}"#, &counter);
    assert!(outcome.applied.is_some());
}

#[test]
fn boolean_tool_use_threshold_uses_the_integer_value() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":true}}"#,
        &counter,
    );
    assert!(outcome.applied.is_some());
}

#[test]
fn only_string_tool_use_ids_count() {
    let messages = vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","name":"no_id"},{"type":"tool_use","id":5,"name":"number_id"},{"type":"tool_use","id":"","name":"empty"},{"type":"tool_use","id":"a","name":"named"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"","content":"empty result"},{"type":"tool_result","tool_use_id":"a","content":"a result"}]}"#,
        ),
    ];
    assert_eq!(tool_ids(&messages).uses, vec!["", "a"]);

    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        messages,
        r#"{"trigger":{"type":"tool_uses","value":1},"keep":{"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
}

#[test]
fn keep_zero_protects_the_latest_completed_result() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(3));
    let Json::List(results) = content(&outcome.messages[1]) else {
        panic!("results");
    };
    assert_eq!(results.len(), 4);
    assert_eq!(
        results[3],
        json(r#"{"type":"tool_result","tool_use_id":"d","content":"result d"}"#)
    );
}

#[test]
fn an_absent_keep_type_keeps_the_requested_count() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(3));
}

#[test]
fn an_explicit_null_keep_type_uses_the_default() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"type":null,"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
}

#[test]
fn a_null_trigger_type_counts_input_tokens() {
    let counter = ScriptedCounter::new(&[100_001]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":null,"value":100001}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 1);
}

#[rstest]
#[case::negative(r#"{"value":-1}"#)]
#[case::float(r#"{"value":2.5}"#)]
#[case::string(r#"{"value":"two"}"#)]
#[case::input_tokens(r#"{"type":"input_tokens","value":0}"#)]
fn invalid_keep_values_use_three(#[case] keep: &str) {
    let counter = ScriptedCounter::new(&[10, 4]);
    let spec_source = format!(r#"{{"trigger":{{"type":"tool_uses","value":0}},"keep":{keep}}}"#);
    let outcome = run_editor(conversation(), &spec_source, &counter);
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(1));
}

#[test]
fn boolean_true_keep_keeps_one() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        generated_conversation(&["a", "b", "c", "d", "e"], &["a", "b", "c", "e", "d"]),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":true}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(3));
}

#[test]
fn boolean_false_keep_keeps_none() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        generated_conversation(&["a", "b", "c", "d", "e"], &["a", "b", "c", "e", "d"]),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":false}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(4));
}

#[test]
fn a_short_history_returns_no_edit() {
    let messages = vec![
        json(r#"{"role":"assistant","content":[{"type":"tool_use","id":"a"}]}"#),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"result"}]}"#,
        ),
    ];
    let counter = ScriptedCounter::new(&[]);
    let outcome = run_editor(
        messages,
        r#"{"trigger":{"type":"tool_uses","value":0}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(counter.call_count(), 0);
}

#[test]
fn unmatched_result_ids_are_untouched() {
    let messages = vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"a"},{"type":"tool_use","id":"b"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"one"},{"type":"tool_result","tool_use_id":"b","content":"two"},{"type":"tool_result","tool_use_id":"z","content":"untouched"}]}"#,
        ),
    ];
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        messages,
        r#"{"trigger":{"type":"tool_uses","value":1},"keep":{"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(2));
    assert_eq!(
        outcome.messages[1],
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"[Cleared by context management]"},{"type":"tool_result","tool_use_id":"b","content":"[Cleared by context management]"},{"type":"tool_result","tool_use_id":"z","content":"untouched"}]}"#
        )
    );
}

#[test]
fn duplicate_results_each_count_as_cleared() {
    let messages = vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"a"},{"type":"tool_use","id":"b"},{"type":"tool_use","id":"c"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"one"},{"type":"tool_result","tool_use_id":"a","content":"two"},{"type":"tool_result","tool_use_id":"b","content":"bee"},{"type":"tool_result","tool_use_id":"c","content":"sea"}]}"#,
        ),
    ];
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        messages,
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":0}}"#,
        &counter,
    );
    assert_eq!(outcome.applied.expect("edit").cleared_tool_uses, Some(3));
}

#[test]
fn selected_ids_without_matching_results_return_no_edit() {
    let messages = vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"a"},{"type":"tool_use","id":"b"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"b","content":"bee"}]}"#,
        ),
    ];
    let counter = ScriptedCounter::new(&[]);
    let outcome = run_editor(
        messages.clone(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":0}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(outcome.messages, messages);
    assert_eq!(counter.call_count(), 0);
}

#[test]
fn cleared_content_matches_the_original_shape() {
    let messages = vec![
        json(
            r#"{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"lookup","input":{"query":"one"}},{"type":"tool_use","id":"b"},{"type":"tool_use","id":"c"},{"type":"tool_use","id":"e"},{"type":"tool_use","id":"d"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"scalar","is_error":true,"cache_control":{"type":"ephemeral"}},{"type":"tool_result","tool_use_id":"b","content":[{"type":"text","text":"listed"}]},{"type":"tool_result","tool_use_id":"c","content":null},{"type":"tool_result","tool_use_id":"e"},{"type":"tool_result","tool_use_id":"d","content":"latest"}],"custom":"kept"}"#,
        ),
    ];
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        messages.clone(),
        r#"{"trigger":{"type":"tool_uses","value":0},"keep":{"value":0}}"#,
        &counter,
    );
    let applied = outcome.applied.expect("edit");
    assert_eq!(applied.cleared_tool_uses, Some(4));
    assert_eq!(outcome.messages[0], messages[0]);
    assert_eq!(
        outcome.messages[1],
        json(
            r#"{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"[Cleared by context management]","is_error":true,"cache_control":{"type":"ephemeral"}},{"type":"tool_result","tool_use_id":"b","content":[{"type":"text","text":"[Cleared by context management]"}]},{"type":"tool_result","tool_use_id":"c","content":"[Cleared by context management]"},{"type":"tool_result","tool_use_id":"e","content":"[Cleared by context management]"},{"type":"tool_result","tool_use_id":"d","content":"latest"}],"custom":"kept"}"#
        )
    );
}

#[test]
fn cleared_input_tokens_subtracts_the_after_count() {
    let counter = ScriptedCounter::new(&[120, 80]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":100}}"#, &counter);
    assert_eq!(
        outcome.applied.expect("edit").cleared_input_tokens,
        Some(40)
    );
}

#[test]
fn cleared_input_tokens_floors_at_zero() {
    let counter = ScriptedCounter::new(&[80, 130]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":70}}"#, &counter);
    assert_eq!(outcome.applied.expect("edit").cleared_input_tokens, Some(0));
}

#[test]
fn input_token_trigger_reuses_the_original_count() {
    let counter = ScriptedCounter::new(&[120, 80]);
    let outcome = run_editor(conversation(), r#"{"trigger":{"value":100}}"#, &counter);
    let calls = counter.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].model, "gpt-4o-mini");
    assert_eq!(calls[0].messages, conversation());
    assert_eq!(calls[0].tools, None);
    assert_eq!(calls[0].count, 120);
    assert_eq!(calls[1].messages, outcome.messages);
    assert_eq!(calls[1].count, 80);
}

#[test]
fn tool_use_trigger_counts_before_only_after_a_replacement() {
    let counter = ScriptedCounter::new(&[100, 40]);
    let outcome = run_editor(
        conversation(),
        r#"{"trigger":{"type":"tool_uses","value":3}}"#,
        &counter,
    );
    let calls = counter.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].messages, conversation());
    assert_eq!(calls[1].messages, outcome.messages);
}

#[test]
fn system_is_accepted_and_never_counted() {
    let edit_spec = spec(r#"{"trigger":{"value":100}}"#);
    let system = json(r#""a system prompt""#);
    let counter = ScriptedCounter::new(&[120, 80]);
    let outcome = apply_clear_tool_uses_20250919(
        EditorRequest {
            model: "gpt-4o-mini",
            messages: conversation(),
            tools: None,
            system: Some(&system),
            edit_spec: &edit_spec,
        },
        &counter,
    )
    .expect("editor");
    assert!(outcome.applied.is_some());
    let calls = counter.calls();
    assert_eq!(calls[0].count, 120);
    assert_eq!(calls[1].count, 80);
}

#[test]
fn ignored_knobs_warn_in_order_even_without_a_trigger() {
    let counter = ScriptedCounter::new(&[10]);
    let outcome = run_editor(
        conversation(),
        r#"{"clear_at_least":null,"exclude_tools":["lookup"],"clear_tool_inputs":false,"trigger":{"value":999999}}"#,
        &counter,
    );
    assert!(outcome.applied.is_none());
    assert_eq!(
        outcome.warnings,
        vec![
            "context_management polyfill: ignoring 'clear_at_least' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)".to_owned(),
            "context_management polyfill: ignoring 'exclude_tools' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)".to_owned(),
            "context_management polyfill: ignoring 'clear_tool_inputs' on clear_tool_uses_20250919 (supported only on Anthropic-family forwarding path in v0)".to_owned(),
        ]
    );
    assert_eq!(counter.call_count(), 1);
}

#[test]
fn applied_edit_lists_ignored_knob_warnings() {
    let counter = ScriptedCounter::new(&[10, 4]);
    let outcome = run_editor(
        conversation(),
        r#"{"clear_at_least":5,"exclude_tools":["lookup"],"clear_tool_inputs":true,"trigger":{"type":"tool_uses","value":0}}"#,
        &counter,
    );
    let applied = outcome.applied.expect("edit");
    assert_eq!(
        applied.warnings,
        Some(vec![
            "clear_at_least_ignored".to_owned(),
            "exclude_tools_ignored".to_owned(),
            "clear_tool_inputs_ignored".to_owned(),
        ])
    );
    assert_eq!(outcome.messages[0], conversation()[0]);
    let Json::List(results) = content(&outcome.messages[1]) else {
        panic!("results");
    };
    assert!(results.iter().any(|result| {
        serde_json::to_string(result)
            .expect("json")
            .contains(CLEARED_TOOL_RESULT_CONTENT)
    }));
}

#[test]
fn cleared_tool_result_content_keeps_the_shape() {
    assert_eq!(
        cleared_tool_result_content(Some(&json(r#""text""#))),
        Json::Text(CLEARED_TOOL_RESULT_CONTENT.to_owned())
    );
    assert_eq!(
        cleared_tool_result_content(Some(&Json::Null)),
        Json::Text(CLEARED_TOOL_RESULT_CONTENT.to_owned())
    );
    assert_eq!(
        cleared_tool_result_content(None),
        Json::Text(CLEARED_TOOL_RESULT_CONTENT.to_owned())
    );
    assert_eq!(
        cleared_tool_result_content(Some(&json(r#"[{"type":"text","text":"x"}]"#))),
        json(r#"[{"type":"text","text":"[Cleared by context management]"}]"#)
    );
}

fn edit(edit_type: &str) -> AppliedEdit {
    AppliedEdit {
        edit_type: Some(edit_type.to_owned()),
        cleared_input_tokens: None,
        cleared_tool_uses: None,
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: None,
    }
}

fn result_with(
    applied_edits: Vec<AppliedEdit>,
    compaction_block: Option<CompactionBlock>,
) -> PolyfillResult {
    PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits,
        compaction_block,
        iterations_usage: None,
    }
}

#[test]
fn polyfill_result_defaults_are_empty() {
    let result = PolyfillResult::default();
    assert!(result.messages.is_empty());
    assert!(result.system.is_none());
    assert!(result.applied_edits.is_empty());
    assert!(result.compaction_block.is_none());
    assert!(result.iterations_usage.is_none());
    assert!(result.applied_edits_for_response().is_none());
}

#[test]
fn compact_edits_without_effects_are_hidden() {
    let result = result_with(
        vec![
            edit(COMPACT_EDIT_TYPE),
            edit(CLEAR_TOOL_USES_EDIT_TYPE),
            AppliedEdit::default(),
        ],
        None,
    );
    assert_eq!(
        result.applied_edits_for_response(),
        Some(vec![
            edit(CLEAR_TOOL_USES_EDIT_TYPE),
            AppliedEdit::default()
        ])
    );
}

#[test]
fn compact_errors_or_warnings_stay_visible() {
    let mut errored = edit(COMPACT_EDIT_TYPE);
    errored.error = Some("summary_call_failed".to_owned());
    let mut warned = edit(COMPACT_EDIT_TYPE);
    warned.warnings = Some(vec!["pause_after_compaction_ignored".to_owned()]);
    let mut empty_error = edit(COMPACT_EDIT_TYPE);
    empty_error.error = Some(String::new());
    let mut empty_warnings = edit(COMPACT_EDIT_TYPE);
    empty_warnings.warnings = Some(Vec::new());
    let result = result_with(
        vec![errored.clone(), warned.clone(), empty_error, empty_warnings],
        None,
    );
    assert_eq!(
        result.applied_edits_for_response(),
        Some(vec![errored, warned])
    );
}

#[test]
fn a_compaction_block_reveals_every_compact_edit() {
    let block = CompactionBlock {
        kind: CompactionKind::Compaction,
        content: Some("summary".to_owned()),
        signature: None,
    };
    let result = result_with(
        vec![edit(COMPACT_EDIT_TYPE), edit(COMPACT_EDIT_TYPE)],
        Some(block),
    );
    assert_eq!(
        result.applied_edits_for_response().map(|edits| edits.len()),
        Some(2)
    );
}

#[test]
fn usage_iterations_keep_their_kind() {
    let iteration = UsageIteration {
        kind: UsageKind::Compaction,
        input_tokens: Some(12),
        output_tokens: Some(3),
    };
    assert_eq!(
        serde_json::to_string(&iteration).expect("json"),
        r#"{"type":"compaction","input_tokens":12,"output_tokens":3}"#
    );
}

#[test]
fn context_management_error_carries_only_status_and_message() {
    let error = ContextManagementError {
        status_code: 403,
        message: "denied".to_owned(),
    };
    assert_eq!(error.status_code, 403);
    assert_eq!(error.message, "denied");
    assert_eq!(error.to_string(), "denied");
}

#[test]
fn context_constant_values() {
    assert_eq!(CLEAR_TOOL_USES_EDIT_TYPE, "clear_tool_uses_20250919");
    assert_eq!(DEFAULT_INPUT_TOKENS_TRIGGER, 100_000);
    assert_eq!(DEFAULT_KEEP_TOOL_USES, 3);
    assert_eq!(
        CLEARED_TOOL_RESULT_CONTENT,
        "[Cleared by context management]"
    );
    assert_eq!(COMPACT_EDIT_TYPE, "compact_20260112");
    assert_eq!(COMPACT_DEFAULT_TRIGGER_TOKENS, 150_000);
    assert_eq!(COMPACT_MIN_TRIGGER_TOKENS, 50_000);
    assert_eq!(COMPACT_SUMMARY_MAX_TOKENS, 4096);
    assert_eq!(
        SUMMARY_MAX_TOKENS_KEY,
        "context_management_summary_max_tokens"
    );
    assert_eq!(SUMMARY_TIMEOUT_SECONDS.to_bits(), 60.0_f64.to_bits());
    assert_eq!(SUMMARY_MODEL_KEY, "context_management_summary_model");
    assert_eq!(
        COMPACT_SUMMARY_SYSTEM_PREFIX,
        "Previous conversation summary: "
    );
    assert_eq!(
        COMPACT_DEFAULT_INSTRUCTIONS,
        "You have written a partial transcript for the initial task above. Please write a summary of the transcript. The purpose of this summary is to provide continuity so you can continue to make progress towards solving the task in a future context, where the raw history above may not be accessible and will be replaced with this summary. Write down anything that would be helpful, including the state, next steps, learnings etc. You must wrap your summary in a <summary></summary> block."
    );
    assert_eq!(
        COMPACT_NO_TOOL_CALLS_SUFFIX,
        " Do not call any tools while writing this summary; respond with text only."
    );
}
