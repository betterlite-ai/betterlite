use betterlite_models::decisions::Json;

use super::super::budget::budget;
use super::super::compactor::{CompactorConfig, ParentInput, compactor_input, compactor_model};
use super::super::contracts::{
    Candidate, Conversation, History, InputBudget, ModelLimits, NativeCapability, OutputAllowance,
    ParentPayload,
};
use super::super::error::{CompactionError, ErrorKind};
use super::super::history::portable_history;
use super::support::{FixedCounter, candidate, deployment, items, messages, request};

const NO_CLOSED_HISTORY: &str =
    "No closed older conversation is available without changing the latest request";
const NO_COMPACTOR: &str = "No configured compactor supports native compaction with enough context and compatible defaults";

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
            any_unsupported_deployment_skips_the_group();
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
        "a_non_text_responses_type_is_unavailable" => a_non_text_responses_type_is_unavailable(),
        "skips_a_group_without_a_known_window" => skips_a_group_without_a_known_window(),
        other => panic!("unknown boundary case {other}"),
    }
}

fn outcome(
    candidates: &[Candidate],
    config_model: Option<&str>,
    needed: i64,
) -> Result<String, CompactionError> {
    let config = CompactorConfig {
        model: config_model,
        max_tokens: 512,
    };
    super::runtime()
        .block_on(compactor_model(
            &config,
            candidates,
            &request(512),
            &FixedCounter { value: needed },
        ))
        .map(|selection| selection.model)
}

fn selected_with_needed(needed: i64) -> Result<String, CompactionError> {
    let candidates = vec![candidate(
        "large",
        vec![deployment(
            true,
            NativeCapability::Supported,
            Some(1000),
            Some(4096),
        )],
    )];
    outcome(&candidates, None, needed)
}

fn selects_at_exactly_the_budget(needed: i64, selects: bool) {
    let result = selected_with_needed(needed);
    assert_eq!(result.is_ok(), selects);
    if selects {
        assert_eq!(result.unwrap(), "large");
    }
}

fn computed(window: i64, max_tokens: i64, ratio: f64) -> InputBudget {
    let limits = ModelLimits {
        max_input_tokens: Some(window),
        max_output_tokens: None,
    };
    let allowance = OutputAllowance {
        max_completion_tokens: None,
        max_output_tokens: None,
        max_tokens: Some(max_tokens),
    };
    budget("parent-model", limits, allowance, ratio).unwrap()
}

fn truncates_the_window_ratio() {
    assert_eq!(
        computed(101, 10, 0.9),
        InputBudget {
            window: Some(101),
            available: Some(80),
        }
    );
}

fn accepts_an_output_allowance_of_one() {
    assert_eq!(computed(10, 1, 0.9).available, Some(8));
}

fn ratio_of_one_keeps_the_window() {
    assert_eq!(computed(100, 10, 1.0).available, Some(90));
}

fn negative_available_is_not_clamped() {
    assert_eq!(computed(100, 100, 0.9).available, Some(-10));
}

fn error_renders_the_reason() {
    let error = CompactionError {
        kind: ErrorKind::Unavailable,
        model: "parent-model".to_owned(),
        reason: NO_CLOSED_HISTORY.to_owned(),
        source: None,
    };
    assert_eq!(
        error.to_string(),
        "Context compaction: No closed older conversation is available without changing the latest request"
    );
}

fn assert_error(error: &CompactionError, kind: ErrorKind, reason: &str) {
    assert_eq!(error.kind, kind);
    assert_eq!(error.model, "parent-model");
    assert_eq!(error.reason, reason);
}

fn unavailable_error_fields() {
    let conversation = messages(r#"[{"role":"user","content":"only"}]"#);
    let error = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap_err();
    assert_error(&error, ErrorKind::Unavailable, NO_CLOSED_HISTORY);
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
    assert_error(
        &error,
        ErrorKind::Rejected,
        "The output allowance must be a positive integer",
    );
}

fn split_skips_a_user_message_holding_a_tool_result() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"f","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]},
            {"role":"user","content":"second"},
            {"role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"f","input":{}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"ok"}]}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.instruction_count, 0);
    assert_eq!(history.prefix_count, 3);
    assert_eq!(history.tail_count, 3);
    assert_eq!(history.calls, vec!["toolu_1".to_owned()]);
    assert_eq!(history.results, vec!["toolu_1".to_owned()]);
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
    assert_eq!(error.reason, NO_CLOSED_HISTORY);
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
    assert_eq!(error.reason, NO_CLOSED_HISTORY);
}

fn any_unsupported_deployment_skips_the_group() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, NativeCapability::Supported, Some(200_000), Some(4096)),
            deployment(
                true,
                NativeCapability::Unsupported,
                Some(200_000),
                Some(4096),
            ),
        ],
    )];
    let error = outcome(&candidates, None, 1).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
}

fn only_the_first_deployment_provider_counts() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, NativeCapability::Supported, Some(200_000), Some(4096)),
            deployment(
                false,
                NativeCapability::Supported,
                Some(200_000),
                Some(4096),
            ),
        ],
    )];
    let selected = outcome(&candidates, None, 1).unwrap();
    assert_eq!(selected, "large");
}

fn every_deployment_budget_is_checked() {
    let candidates = vec![candidate(
        "large",
        vec![
            deployment(true, NativeCapability::Supported, Some(200_000), Some(4096)),
            deployment(true, NativeCapability::Supported, Some(1000), Some(4096)),
        ],
    )];
    let error = outcome(&candidates, None, 500).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
}

fn configured_model_absent_from_candidates() {
    let candidates: Vec<Candidate> = vec![candidate(
        "large",
        vec![deployment(
            true,
            NativeCapability::Supported,
            Some(200_000),
            Some(4096),
        )],
    )];
    let error = outcome(&candidates, Some("missing"), 1).unwrap_err();
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
    let summary = compactor_input(&ParentInput::Chat(&payload), &history, 4096).unwrap();
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

fn a_non_text_responses_type_is_unavailable() {
    let conversation = items(
        r#"[{"type":42,"role":"user","content":[{"type":"input_text","text":"first"}]},
            {"role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    );
    let tools: Vec<Json> = items("[]");
    let error = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(
        error.reason,
        "Opaque or provider-managed Responses items require client-managed native compaction"
    );
}

fn skips_a_group_without_a_known_window() {
    let candidates = vec![candidate(
        "large",
        vec![deployment(
            true,
            NativeCapability::Supported,
            None,
            Some(4096),
        )],
    )];
    let empty = outcome(&candidates, None, 0).unwrap_err();
    assert_error(&empty, ErrorKind::Unavailable, NO_COMPACTOR);
    let full = outcome(&candidates, None, 500).unwrap_err();
    assert_error(&full, ErrorKind::Unavailable, NO_COMPACTOR);
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
