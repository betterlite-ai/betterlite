use betterlite_models::decisions::Json;
use betterlite_openai::chat::Tool;

use super::super::budget::budget;
use super::super::compactor::{
    CompactorConfig, compactor_input, compactor_model, responses_compactor_input,
};
use super::super::contracts::{
    Candidate, Conversation, History, InputBudget, ModelLimits, OutputAllowance, ParentPayload,
    ResponsesPayload,
};
use super::super::error::ErrorKind;
use super::super::history::{portable_history, tool_ids};
use super::support::{
    CheckedConverter, FailingCounter, FixedCounter, RecordingCounter, candidate, deployment, items,
    json, messages, request,
};

pub(super) fn run(name: &str) {
    match name {
        "keeps_a_closed_tool_pair" => keeps_a_closed_tool_pair(),
        "keeps_block_tool_pairs" => keeps_block_tool_pairs(),
        "keeps_responses_function_pairs" => keeps_responses_function_pairs(),
        "treats_mid_conversation_instructions_as_unavailable" => {
            treats_mid_conversation_instructions_as_unavailable();
        }
        "treats_a_text_input_as_unavailable" => treats_a_text_input_as_unavailable(),
        "treats_an_open_tool_call_as_unavailable" => treats_an_open_tool_call_as_unavailable(),
        "treats_an_unclosed_tool_result_as_unavailable" => {
            treats_an_unclosed_tool_result_as_unavailable();
        }
        "treats_an_empty_tool_id_as_unavailable" => treats_an_empty_tool_id_as_unavailable(),
        "treats_duplicate_tool_calls_as_unavailable" => {
            treats_duplicate_tool_calls_as_unavailable();
        }
        "treats_an_empty_prefix_as_unavailable" => treats_an_empty_prefix_as_unavailable(),
        "treats_unknown_responses_items_as_unavailable" => {
            treats_unknown_responses_items_as_unavailable();
        }
        "treats_non_text_responses_blocks_as_unavailable" => {
            treats_non_text_responses_blocks_as_unavailable();
        }
        "treats_non_function_tools_as_unavailable" => treats_non_function_tools_as_unavailable(),
        "treats_thinking_blocks_as_unavailable" => treats_thinking_blocks_as_unavailable(),
        "treats_chat_thinking_blocks_as_unavailable" => {
            treats_chat_thinking_blocks_as_unavailable();
        }
        "keeps_empty_thinking_blocks" => keeps_empty_thinking_blocks(),
        "treats_missing_responses_type_as_message" => treats_missing_responses_type_as_message(),
        "budget_prefers_max_completion_tokens" => budget_case(
            OutputAllowance {
                max_completion_tokens: Some(7),
                max_output_tokens: Some(9),
                max_tokens: Some(11),
            },
            Some(50),
        ),
        "budget_prefers_max_output_tokens" => budget_case(
            OutputAllowance {
                max_completion_tokens: None,
                max_output_tokens: Some(9),
                max_tokens: Some(11),
            },
            Some(50),
        ),
        "budget_prefers_payload_max_tokens" => budget_case(
            OutputAllowance {
                max_completion_tokens: None,
                max_output_tokens: None,
                max_tokens: Some(11),
            },
            Some(50),
        ),
        "budget_falls_back_to_model_output" => budget_case(
            OutputAllowance {
                max_completion_tokens: None,
                max_output_tokens: None,
                max_tokens: None,
            },
            Some(11),
        ),
        "budget_has_no_available_without_window" => {
            let limits = ModelLimits {
                max_input_tokens: None,
                max_output_tokens: Some(11),
            };
            let allowance = OutputAllowance {
                max_completion_tokens: Some(7),
                max_output_tokens: None,
                max_tokens: None,
            };
            let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
            assert_eq!(
                computed,
                InputBudget {
                    window: None,
                    available: None,
                }
            );
        }
        "budget_has_no_available_without_output" => {
            let limits = ModelLimits {
                max_input_tokens: Some(1000),
                max_output_tokens: None,
            };
            let allowance = OutputAllowance {
                max_completion_tokens: None,
                max_output_tokens: None,
                max_tokens: None,
            };
            let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
            assert_eq!(
                computed,
                InputBudget {
                    window: Some(1000),
                    available: None,
                }
            );
        }
        "budget_rejects_a_zero_output_allowance" => {
            budget_error_case(Some(0), "The output allowance must be a positive integer");
        }
        "budget_rejects_a_negative_output_allowance" => {
            budget_error_case(Some(-3), "The output allowance must be a positive integer");
        }
        "compactor_input_keeps_tools_system_and_user" => compactor_input_keeps_extra_keys(),
        "selects_the_configured_group" => selects_the_configured_group(),
        "selects_the_first_eligible_group" => selects_the_first_eligible_group(),
        "skips_a_group_without_a_provider" => skips_a_group_without_a_provider(),
        "skips_a_group_that_lacks_support" => skips_a_group_that_lacks_support(),
        "skips_a_group_with_incompatible_defaults" => skips_a_group_with_incompatible_defaults(),
        "skips_a_group_that_cannot_hold_the_summary" => {
            skips_a_group_that_cannot_hold_the_summary();
        }
        "responses_compactor_input_uses_the_older_history" => {
            responses_compactor_input_uses_the_older_history();
        }
        "counts_the_summary_request" => counts_the_summary_request(),
        "propagates_a_counter_failure" => propagates_a_counter_failure(),
        other => panic!("unknown behavior case {other}"),
    }
}

fn keeps_a_closed_tool_pair() {
    let conversation = messages(
        r#"[
            {"role":"system","content":"You are terse."},
            {"role":"developer","content":"Follow the schema."},
            {"role":"user","content":"first"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"call_1","content":"42"},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let history = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap();
    assert_eq!(
        history,
        History {
            instruction_count: 2,
            prefix_count: 3,
            tail_count: 1,
            calls: vec!["call_1".to_owned()],
            results: vec!["call_1".to_owned()],
        }
    );
    assert_eq!(
        tool_ids(&conversation, "parent-model", false).unwrap(),
        vec!["call_1".to_owned()]
    );
    assert_eq!(
        tool_ids(&conversation, "parent-model", true).unwrap(),
        vec!["call_1".to_owned()]
    );
}

fn keeps_block_tool_pairs() {
    let conversation = messages(
        r#"[
            {"role":"user","content":[{"type":"text","text":"go"}]},
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
    assert_eq!(history.calls, vec!["toolu_1".to_owned()]);
    assert_eq!(history.results, vec!["toolu_1".to_owned()]);
}

fn keeps_responses_function_pairs() {
    let conversation = items(
        r#"[
            {"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},
            {"type":"function_call","call_id":"call_1","name":"f","arguments":"{}"},
            {"type":"function_call_output","call_id":"call_1","output":"ok"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}
        ]"#,
    );
    let tools: Vec<Json> = items("[]");
    let history = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.instruction_count, 0);
    assert_eq!(history.prefix_count, 3);
    assert_eq!(history.tail_count, 1);
    assert_eq!(history.calls, vec!["call_1".to_owned()]);
    assert_eq!(history.results, vec!["call_1".to_owned()]);
}

fn treats_mid_conversation_instructions_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"system","content":"late instruction"},
            {"role":"user","content":"latest"}
        ]"#,
    );
    let error = portable_history(
        &Conversation::Messages(conversation.as_slice()),
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
    assert_eq!(
        error.reason,
        "Mid-conversation instructions cannot be compacted"
    );
}

fn treats_a_text_input_as_unavailable() {
    let error = portable_history(&Conversation::Text("hello"), "parent-model").unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(
        error.reason,
        "A single user input cannot be compacted without changing the latest request"
    );
}

fn treats_an_open_tool_call_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}]},
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

fn treats_an_unclosed_tool_result_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"tool","tool_call_id":"call_1","content":"late"},
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

fn treats_an_empty_tool_id_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","tool_calls":[{"id":"","type":"function","function":{"name":"f","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"","content":"late"},
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

fn treats_duplicate_tool_calls_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}},
                {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"}}
            ]},
            {"role":"tool","tool_call_id":"call_1","content":"first result"},
            {"role":"tool","tool_call_id":"call_1","content":"second result"},
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

fn treats_an_empty_prefix_as_unavailable() {
    let conversation = messages(r#"[{"role":"user","content":"only"}]"#);
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

fn treats_unknown_responses_items_as_unavailable() {
    let conversation = items(r#"[{"type":"web_search_call","id":"ws_1"}]"#);
    let tools: Vec<Json> = items("[]");
    let error = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(
        error.reason,
        "Opaque or provider-managed Responses items require client-managed native compaction"
    );
}

fn treats_non_text_responses_blocks_as_unavailable() {
    let conversation = items(
        r#"[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"x"}]}]"#,
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
    assert_eq!(
        error.reason,
        "Only text history and ordinary function tools support portable Responses compaction"
    );
}

fn treats_non_function_tools_as_unavailable() {
    let conversation = items(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    );
    let tools = items(r#"[{"type":"web_search"}]"#);
    let error = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap_err();
    assert_eq!(
        error.reason,
        "Only text history and ordinary function tools support portable Responses compaction"
    );
}

fn treats_thinking_blocks_as_unavailable() {
    let conversation = items(
        r#"[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"x"}],
             "thinking_blocks":[{"type":"thinking","signature":"sig"}]}]"#,
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
    assert_eq!(
        error.reason,
        "Native reasoning or compaction blocks require client-managed native compaction"
    );
}

fn treats_chat_thinking_blocks_as_unavailable() {
    let conversation = messages(
        r#"[
            {"role":"user","content":"first"},
            {"role":"assistant","content":[{"type":"thinking","thinking":"hm","signature":"sig"}]},
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
        "Native reasoning or compaction blocks require client-managed native compaction"
    );
}

fn keeps_empty_thinking_blocks() {
    let conversation = items(
        r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"first"}],"thinking_blocks":[]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    );
    let tools: Vec<Json> = items("[]");
    let history = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.prefix_count, 1);
    assert_eq!(history.tail_count, 1);
}

fn treats_missing_responses_type_as_message() {
    let conversation = items(
        r#"[{"role":"user","content":[{"type":"input_text","text":"first"}]},
            {"role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    );
    let tools: Vec<Json> = items("[]");
    let history = portable_history(
        &Conversation::Responses {
            items: conversation.as_slice(),
            tools: tools.as_slice(),
        },
        "parent-model",
    )
    .unwrap();
    assert_eq!(history.prefix_count, 1);
}

fn budget_case(allowance: OutputAllowance, model_output: Option<i64>) {
    let limits = ModelLimits {
        max_input_tokens: Some(1000),
        max_output_tokens: model_output,
    };
    let computed = budget("parent-model", limits, allowance, 0.9).unwrap();
    assert_eq!(computed.window, Some(1000));
    let output = allowance
        .max_completion_tokens
        .or(allowance.max_output_tokens)
        .or(allowance.max_tokens)
        .or(model_output)
        .unwrap_or_default();
    assert_eq!(computed.available, Some(900_i64.saturating_sub(output)));
}

fn budget_error_case(output: Option<i64>, reason: &str) {
    let limits = ModelLimits {
        max_input_tokens: Some(1000),
        max_output_tokens: Some(11),
    };
    let allowance = OutputAllowance {
        max_completion_tokens: output,
        max_output_tokens: None,
        max_tokens: None,
    };
    let error = budget("parent-model", limits, allowance, 0.9).unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert_eq!(error.model, "parent-model");
    assert_eq!(error.reason, reason);
}

fn compactor_input_keeps_extra_keys() {
    let conversation = messages(
        r#"[{"role":"system","content":"sys"},
            {"role":"user","content":"first"},
            {"role":"assistant","content":"answer"},
            {"role":"user","content":"latest"}]"#,
    );
    let tools: Vec<Tool> =
        serde_json::from_str(r#"[{"type":"function","function":{"name":"f","arguments":"{}"}}]"#)
            .unwrap();
    let system = json("\"system prompt\"");
    let user = json("\"caller\"");
    let payload = ParentPayload {
        model: "parent-model",
        messages: conversation.as_slice(),
        tools: Some(&tools),
        system: Some(&system),
        user: Some(&user),
    };
    let history = History {
        instruction_count: 1,
        prefix_count: 2,
        tail_count: 1,
        calls: Vec::new(),
        results: Vec::new(),
    };
    let summary = compactor_input(&payload, &history, 4096);
    assert_eq!(summary.model, "parent-model");
    assert_eq!(summary.max_tokens, 4096);
    assert_eq!(summary.messages.as_slice(), &conversation[..3]);
    assert_eq!(summary.tools.as_deref(), Some(tools.as_slice()));
    assert_eq!(summary.system, Some(system));
    assert_eq!(summary.user, Some(user));
}

fn compactor_config(model: Option<&str>) -> CompactorConfig<'_> {
    CompactorConfig {
        model,
        max_tokens: 512,
    }
}

fn selects_the_configured_group() {
    let candidates = vec![
        candidate(
            "small",
            vec![deployment(true, true, true, Some(200_000), Some(4096))],
        ),
        candidate(
            "large",
            vec![deployment(true, true, true, Some(200_000), Some(4096))],
        ),
    ];
    let selected = super::runtime()
        .block_on(compactor_model(
            &compactor_config(Some("large")),
            &candidates,
            &request(512),
            &FixedCounter { value: 10 },
        ))
        .unwrap();
    assert_eq!(selected.model, "large");
}

fn selects_the_first_eligible_group() {
    let candidates = vec![
        candidate(
            "small",
            vec![deployment(false, true, true, Some(200_000), Some(4096))],
        ),
        candidate(
            "large",
            vec![deployment(true, true, true, Some(200_000), Some(4096))],
        ),
    ];
    let selected = super::runtime()
        .block_on(compactor_model(
            &compactor_config(None),
            &candidates,
            &request(512),
            &FixedCounter { value: 10 },
        ))
        .unwrap();
    assert_eq!(selected.model, "large");
}

fn skip_case(candidates: Vec<Candidate>) {
    let error = super::runtime()
        .block_on(compactor_model(
            &compactor_config(None),
            &candidates,
            &request(512),
            &FixedCounter { value: 10 },
        ))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unavailable);
    assert_eq!(error.model, "parent-model");
    assert_eq!(
        error.reason,
        "No configured compactor supports native compaction with enough context and compatible defaults"
    );
}

fn skips_a_group_without_a_provider() {
    skip_case(vec![candidate(
        "small",
        vec![deployment(false, true, true, Some(200_000), Some(4096))],
    )]);
}

fn skips_a_group_that_lacks_support() {
    skip_case(vec![candidate(
        "small",
        vec![deployment(true, false, true, Some(200_000), Some(4096))],
    )]);
}

fn skips_a_group_with_incompatible_defaults() {
    skip_case(vec![candidate(
        "small",
        vec![deployment(true, true, false, Some(200_000), Some(4096))],
    )]);
}

fn skips_a_group_that_cannot_hold_the_summary() {
    skip_case(vec![candidate(
        "small",
        vec![deployment(true, true, true, Some(1000), Some(4096))],
    )]);
}

fn responses_compactor_input_uses_the_older_history() {
    let conversation = items(
        r#"[{"type":"message","role":"system","content":[{"type":"input_text","text":"sys"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"first"}]},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"latest"}]}]"#,
    );
    let tools = items(r#"[{"type":"function","name":"f"}]"#);
    let request_payload = json(r#"{"model":"parent-model"}"#);
    let payload = ResponsesPayload {
        model: "parent-model",
        items: conversation.as_slice(),
        tools: tools.as_slice(),
        request: &request_payload,
        system: None,
        user: None,
    };
    let history = History {
        instruction_count: 1,
        prefix_count: 2,
        tail_count: 1,
        calls: Vec::new(),
        results: Vec::new(),
    };
    let converter = CheckedConverter {
        older_items: 3,
        tools: 1,
    };
    let summary = responses_compactor_input(&payload, &history, 4096, &converter).unwrap();
    assert_eq!(
        summary.messages,
        messages(r#"[{"role":"user","content":"older"}]"#)
    );
    assert_eq!(summary.tools, Some(Vec::new()));
    assert_eq!(summary.max_tokens, 4096);
}

fn counts_the_summary_request() {
    let counter = RecordingCounter {
        seen: std::sync::Mutex::new(Vec::new()),
    };
    let summary = request(512);
    let candidates = vec![candidate(
        "large",
        vec![deployment(true, true, true, Some(200_000), Some(4096))],
    )];
    let selected = super::runtime()
        .block_on(compactor_model(
            &compactor_config(None),
            &candidates,
            &summary,
            &counter,
        ))
        .unwrap();
    assert_eq!(selected.model, "large");
    let seen = counter.seen.lock().unwrap();
    assert_eq!(seen.as_slice(), &[summary]);
}

fn propagates_a_counter_failure() {
    let candidates = vec![candidate(
        "large",
        vec![deployment(true, true, true, Some(200_000), Some(4096))],
    )];
    let error = super::runtime()
        .block_on(compactor_model(
            &compactor_config(None),
            &candidates,
            &request(512),
            &FailingCounter,
        ))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Rejected);
    assert_eq!(error.reason, "the counter failed");
}
