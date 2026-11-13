use betterlite_models::decisions::Json;

use super::super::budget::budget;
use super::super::compactor::{CompactorConfig, compactor_input, compactor_model};
use super::super::contracts::{
    Candidate, Conversation, History, InputBudget, ModelLimits, OutputAllowance, ParentPayload,
};
use super::super::error::{CompactionError, ErrorKind};
use super::super::history::portable_history;
use super::support::{FixedCounter, candidate, deployment, items, messages, request};

pub(super) fn run(name: &str) {
    match name {
        "selects_at_exactly_the_budget" => selects_at_exactly_the_budget(388, true),
        "skips_one_token_over_the_budget" => selects_at_exactly_the_budget(389, false),
        "truncates_the_window_ratio" => truncates_the_window_ratio(),
        "accepts_an_output_allowance_of_one" => accepts_an_output_allowance_of_one(),
        "ratio_of_one_keeps_the_window" => ratio_of_one_keeps_the_window(),
        "negative_available_is_not_clamped" => negative_available_is_not_clamped(),
        "error_renders_the_reason" => error_renders_the_reason(),
        "unavailable_error_fields" => unavailable_error_fields(),
        "rejected_error_fields" => rejected_error_fields(),
        "split_skips_a_user_message_holding_a_tool_result" => {
            split_skips_a_user_message_holding_a_tool_result();
        }
        "an_empty_tool_result_id_is_unavailable" => an_empty_tool_result_id_is_unavailable(),
        "duplicate_results_without_calls_are_unavailable" => {
            duplicate_results_without_calls_are_unavailable();
        }
        "any_unsupported_deployment_skips_the_group" => {
            any_unsupported_deployment_skips_the_group()
        }
        "only_the_first_deployment_provider_counts" => only_the_first_deployment_provider_counts(),
        "every_deployment_budget_is_checked" => every_deployment_budget_is_checked(),
        "configured_model_absent_from_candidates" => configured_model_absent_from_candidates(),
        "compactor_input_slices_the_older_messages" => compactor_input_slices_the_older_messages(),
        "call_ids_are_sorted_not_order_bound" => call_ids_are_sorted_not_order_bound(),
        "keeps_an_open_tail_tool_call" => keeps_an_open_tail_tool_call(),
        "counts_numeric_thinking_blocks_by_truthiness" => {
            counts_numeric_thinking_blocks_by_truthiness();
        }
        "counts_text_and_object_thinking_blocks_by_truthiness" => {
            counts_text_and_object_thinking_blocks_by_truthiness();
        }
        other => panic!("unknown boundary case {other}"),
    }
}

fn selected_with_needed(needed: i64) -> Result<String, CompactionError> {
    let candidates = vec![candidate(
        "large",
        vec![deployment(true, true, true, Some(1000), Some(4096))],
    )];
    let config = CompactorConfig {
        model: None,
        max_tokens: 512,
    };
    super::runtime()
        .block_on(compactor_model(
            &config,
            &candidates,
            &request(512),
            &FixedCounter { value: needed },
        ))
        .map(|selection| selection.model)
}

fn selects_at_exactly_the_budget(needed: i64, selects: bool) {
    let outcome = selected_with_needed(needed);
    assert_eq!(outcome.is_ok(), selects);
    if selects {
        assert_eq!(outcome.unwrap(), "large");
    }
}

fn truncates_the_window_ratio() {
    let limits = ModelLimits {
        max_input_tokens: Some(101),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: None,
        max_output_tokens: None,
        max_tokens: Some(10),
    };
    let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
    assert_eq!(
        computed,
        InputBudget {
            window: Some(101),
            available: Some(80),
        }
    );
}

fn accepts_an_output_allowance_of_one() {
    let limits = ModelLimits {
        max_input_tokens: Some(10),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: None,
        max_output_tokens: None,
        max_tokens: Some(1),
    };
    let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
    assert_eq!(computed.available, Some(8));
}

fn ratio_of_one_keeps_the_window() {
    let limits = ModelLimits {
        max_input_tokens: Some(100),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: None,
        max_output_tokens: None,
        max_tokens: Some(10),
    };
    let computed = budget("parent-model", limits, allowance, 1.0).unwrap();
    assert_eq!(computed.available, Some(90));
}

fn negative_available_is_not_clamped() {
    let limits = ModelLimits {
        max_input_tokens: Some(100),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: None,
        max_output_tokens: None,
        max_tokens: Some(100),
    };
    let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
    assert_eq!(computed.available, Some(-10));
}

fn error_renders_the_reason() {
    let error = CompactionError {
        kind: ErrorKind::Unavailable,
        model: "parent-model".to_owned(),
        reason: "No closed older conversation is available without changing the latest request"
            .to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "Context compaction: No closed older conversation is available without changing the latest request"
    );
}

fn unavailable_error_fields() {
    let conversation = messages(r#"[{"role":"user","content":"only"}]"#);
    let error = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
    assert_eq!(
        error.reason,
        "No closed older conversation is available without changing the latest request"
    );
}

fn rejected_error_fields() {
    let limits = ModelLimits {
        max_input_tokens: Some(1000),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: Some(0),
        max_output_tokens: None,
        max_tokens: None,
    };
    let error = budget("parent-model", limits, allowance, 0.9).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert_eq!(error.model, "parent-model");
    assert_eq!(
        error.reason,
        "The output allowance must be a positive integer"
    );
}

fn split_skips_a_user_message_holding_a_tool_result() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"f","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.instruction_count, 0);
    assert_eq!(history.prefix_count, 3);
    assert_eq!(history.tail_count, 1);
}

fn an_empty_tool_result_id_is_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"f","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"","content":"ok"}]},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let error = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(
        error.reason,
        "No closed older conversation is available without changing the latest request"
    );
}

fn duplicate_results_without_calls_are_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"tool","tool_call_id":"call_1","content":"one"},
            {"role":"tool","tool_call_id":"call_1","content":"two"},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let error = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(
        error.reason,
        "No closed older conversation is available without changing the latest request"
    );
}

fn any_unsupported_deployment_skips_the_group() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, true, true, Some(200_000), Some(4096)),
            deployment(true, false, true, Some(200_000), Some(4096)),
        ],
    )];
    let config = CompactorConfig {
        model: None,
        max_tokens: 512,
    };
    let error = super::runtime()
        .block_on(compactor_model(
            &config,
            &candidates,
            &request(512),
            &FixedCounter { value: 1 },
        ))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
}

fn only_the_first_deployment_provider_counts() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, true, true, Some(200_000), Some(4096)),
            deployment(false, true, true, Some(200_000), Some(4096)),
        ],
    )];
    let config = CompactorConfig {
        model: None,
        max_tokens: 512,
    };
    let selected = super::runtime()
        .block_on(compactor_model(
            &config,
            &candidates,
            &request(512),
            &FixedCounter { value: 1 },
        ))
        .unwrap();
    assert_eq!(selected.model, "large");
}

fn every_deployment_budget_is_checked() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, true, true, Some(200_000), Some(4096)),
            deployment(true, true, true, Some(1000), Some(4096)),
        ],
    )];
    let config = CompactorConfig {
        model: None,
        max_tokens: 512,
    };
    let error = super::runtime()
        .block_on(compactor_model(
            &config,
            &candidates,
            &request(512),
            &FixedCounter { value: 500 },
        ))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
}

fn configured_model_absent_from_candidates() {
    let candidates: Vec<Candidate> = vec![candidate(
        "large",
        vec![deployment(true, true, true, Some(200_000), Some(4096))],
    )];
    let config = CompactorConfig {
        model: Some("missing"),
        max_tokens: 512,
    };
    let error = super::runtime()
        .block_on(compactor_model(
            &config,
            &candidates,
            &request(512),
            &FixedCounter { value: 1 },
        ))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
}

fn compactor_input_slices_the_older_messages() {
    let conversation = messages(
        r#"[{"role":"system","content":"sys"},
            {"role":"user","content":"first"},
            {"role":"assistant","content":"answer"},
            {"role":"tool","tool_call_id":"call_1","content":"result"},
            {"role":"user","content":"latest"}]"#,
    );
    let payload = ParentPayload {
        model: "parent-model",
        messages: conversation.as_slice(),
        tools: None,
        system: None,
        user: None,
    };
    let history = History {
        instruction_count: 1,
        prefix_count: 3,
        tail_count: 1,
        calls: vec!["call_1".to_owned()],
        results: vec!["call_1".to_owned()],
    };
    let summary = compactor_input(&payload, &history, 4096);
    assert_eq!(summary.messages.as_slice(), &conversation[..4]);
}

fn call_ids_are_sorted_not_order_bound() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","tool_calls":[
                {"id":"call_b","type":"function","function":{"name":"f","arguments":"{}"}},
                {"id":"call_a","type":"function","function":{"name":"g","arguments":"{}"}}
            ]},
            {"role":"tool","tool_call_id":"call_a","content":"a"},
            {"role":"tool","tool_call_id":"call_b","content":"b"},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(
        history.calls,
        vec!["call_b".to_owned(), "call_a".to_owned()]
    );
    assert_eq!(
        history.results,
        vec!["call_a".to_owned(), "call_b".to_owned()]
    );
    assert_eq!(history.prefix_count, 4);
}

fn keeps_an_open_tail_tool_call() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"user","content":"latest"},
            {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.prefix_count, 1);
    assert_eq!(history.tail_count, 2);
    assert!(history.calls.is_empty());
}

fn responses_history_with_thinking(raw: &str) -> Result<History, CompactionError> {
    let conversation = items(raw);
    let tools: Vec<Json> = items("[]");
    portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
}

fn counts_numeric_thinking_blocks_by_truthiness() {
    let present = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":1},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap_err();
    assert_eq!(
        present.reason,
        "Native reasoning or compaction blocks require client-managed native compaction"
    );
    let absent = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":0},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap();
    assert_eq!(absent.prefix_count, 1);
}

fn counts_text_and_object_thinking_blocks_by_truthiness() {
    let text = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":"sig"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap_err();
    assert_eq!(
        text.reason,
        "Native reasoning or compaction blocks require client-managed native compaction"
    );
    let empty_text = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":""},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap();
    assert_eq!(empty_text.prefix_count, 1);
    let object = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":{"type":"thinking"}},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap_err();
    assert_eq!(
        object.reason,
        "Native reasoning or compaction blocks require client-managed native compaction"
    );
    let empty_object = responses_history_with_thinking(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":{}},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    )
    .unwrap();
    assert_eq!(empty_object.prefix_count, 1);
}
