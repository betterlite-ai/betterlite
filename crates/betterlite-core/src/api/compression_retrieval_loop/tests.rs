#![cfg(test)]

use betterlite_context::agentic_callback::AgenticLogging;
use betterlite_models::decisions::Json;

use crate::api::compression_interception::tools::CONTENT_RETRIEVE_TOOL_NAME;

use super::CompressionRetrievalLoop;
use super::gate::extract_retrieval_blocks;
use super::plan::{RetrievalLoopError, followup_kwargs, retrieval_content};
use support::{
    MODEL, NOW, cache_map, followups, index, interception, interception_at, list, logging,
    loop_tools, manual_clock, number, object, other_tools, plan, raw_json, raw_map, request,
    response, retrieval_block, retrieval_call, retrieval_tools, text, tool_result, tool_results,
    tool_use,
};

mod support;

#[test]
fn gate_disabled_returns_false_and_empty() {
    let interception = interception(false);
    let response = response(vec![retrieval_block(Some("call-1"), None)]);
    let (run, ready) = interception.should_run_agentic_loop(&response, Some(&retrieval_tools()));
    assert!(!run);
    assert!(ready.is_empty());
}

#[test]
fn gate_without_a_retrieval_tool_returns_false_and_empty() {
    let interception = interception(true);
    let response = response(vec![retrieval_block(Some("call-1"), None)]);
    let (run, ready) = interception.should_run_agentic_loop(&response, None);
    assert!(!run);
    assert!(ready.is_empty());
    let (other_run, other_ready) =
        interception.should_run_agentic_loop(&response, Some(&other_tools()));
    assert!(!other_run);
    assert!(other_ready.is_empty());
}

#[test]
fn gate_without_a_retrieval_call_returns_false_and_empty() {
    let interception = interception(true);
    let tools = retrieval_tools();
    let text_only = response(vec![object(vec![
        ("type", text("text")),
        ("text", text("hi")),
    ])]);
    assert!(
        !interception
            .should_run_agentic_loop(&text_only, Some(&tools))
            .0
    );
    let other_call = response(vec![object(vec![
        ("type", text("tool_use")),
        ("id", text("call-1")),
        ("name", text("other_tool")),
        ("input", object(Vec::new())),
    ])]);
    assert!(
        !interception
            .should_run_agentic_loop(&other_call, Some(&tools))
            .0
    );
    let no_content = object(vec![("type", text("message"))]);
    assert!(
        !interception
            .should_run_agentic_loop(&no_content, Some(&tools))
            .0
    );
}

#[test]
fn gate_returns_run_and_the_map_in_key_order() {
    let interception = interception(true);
    let thinking = object(vec![
        ("type", text("thinking")),
        ("thinking", text("hm")),
        ("signature", text("sig")),
    ]);
    let redacted = object(vec![
        ("type", text("redacted_thinking")),
        ("data", text("secret")),
    ]);
    let response = response(vec![
        thinking.clone(),
        retrieval_block(Some("call-1"), Some(object(vec![("key", text("k"))]))),
        redacted.clone(),
    ]);
    let (run, ready) = interception.should_run_agentic_loop(&response, Some(&retrieval_tools()));
    assert!(run);
    assert_eq!(
        ready.keys().map(String::as_str).collect::<Vec<_>>(),
        ["tool_calls", "thinking_blocks", "tool_type"]
    );
    assert_eq!(
        ready.get("tool_calls"),
        Some(&list(vec![retrieval_call("call-1", Some("k"))]))
    );
    assert_eq!(
        ready.get("thinking_blocks"),
        Some(&list(vec![thinking, redacted]))
    );
    assert_eq!(ready.get("tool_type"), Some(&text("compression_retrieval")));
}

#[test]
fn extract_rebuilds_retrieval_calls_in_content_order() {
    let first = object(vec![
        ("type", text("tool_use")),
        ("id", text("call-1")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", object(vec![("key", text("k"))])),
        ("cache_control", object(vec![("type", text("ephemeral"))])),
    ]);
    let second = object(vec![
        ("type", text("tool_use")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
    ]);
    let (tool_calls, thinking_blocks) = extract_retrieval_blocks(&response(vec![second, first]));
    let unnamed = object(vec![
        ("id", Json::Null),
        ("type", text("tool_use")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", object(Vec::new())),
    ]);
    assert_eq!(
        tool_calls,
        vec![unnamed, retrieval_call("call-1", Some("k"))]
    );
    assert!(thinking_blocks.is_empty());
}

#[test]
fn extract_keeps_thinking_blocks_whole() {
    let thinking = object(vec![
        ("type", text("thinking")),
        ("thinking", text("hm")),
        ("signature", text("sig")),
        ("extra", number(1)),
    ]);
    let redacted = object(vec![
        ("type", text("redacted_thinking")),
        ("data", text("secret")),
    ]);
    let (tool_calls, thinking_blocks) =
        extract_retrieval_blocks(&response(vec![thinking.clone(), redacted.clone()]));
    assert!(tool_calls.is_empty());
    assert_eq!(thinking_blocks, vec![thinking, redacted]);
}

#[test]
fn extract_returns_empty_lists_for_unreadable_content() {
    let no_content = object(vec![("type", text("message"))]);
    assert_eq!(
        extract_retrieval_blocks(&no_content),
        (Vec::new(), Vec::new())
    );
    let scalar_content = object(vec![("content", text("hi"))]);
    assert_eq!(
        extract_retrieval_blocks(&scalar_content),
        (Vec::new(), Vec::new())
    );
    assert_eq!(
        extract_retrieval_blocks(&text("hi")),
        (Vec::new(), Vec::new())
    );
    let blocks = response(vec![
        text("not a block"),
        object(vec![("name", text(CONTENT_RETRIEVE_TOOL_NAME))]),
        object(vec![
            ("type", number(4)),
            ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ]),
        object(vec![("type", text("tool_use")), ("name", number(4))]),
        object(vec![
            ("type", text("tool_use")),
            ("name", text("other_tool")),
        ]),
        object(vec![
            ("type", text("text")),
            ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ]),
    ]);
    assert_eq!(extract_retrieval_blocks(&blocks), (Vec::new(), Vec::new()));
}

#[test]
fn extract_defaults_a_missing_or_null_input_to_an_empty_object() {
    let missing = object(vec![
        ("type", text("tool_use")),
        ("id", text("call-1")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
    ]);
    let null = object(vec![
        ("type", text("tool_use")),
        ("id", text("call-2")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", Json::Null),
    ]);
    let (tool_calls, _thinking) = extract_retrieval_blocks(&response(vec![missing, null]));
    let empty = object(Vec::new());
    let first = object(vec![
        ("id", text("call-1")),
        ("type", text("tool_use")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", empty.clone()),
    ]);
    let second = object(vec![
        ("id", text("call-2")),
        ("type", text("tool_use")),
        ("name", text(CONTENT_RETRIEVE_TOOL_NAME)),
        ("input", empty),
    ]);
    assert_eq!(tool_calls, vec![first, second]);
}

#[test]
fn resolve_returns_the_cached_string_and_the_two_fallbacks() {
    let cache = cache_map(&[("k", "cached text")]);
    let call = retrieval_call("call-1", Some("k"));
    assert_eq!(
        retrieval_content(&call, &cache).expect("resolves"),
        "cached text"
    );
    let unknown = retrieval_call("call-1", Some("gone"));
    assert_eq!(
        retrieval_content(&unknown, &cache).expect("resolves"),
        "[compressed content key 'gone' not found]"
    );
    let no_key = retrieval_call("call-1", None);
    assert_eq!(
        retrieval_content(&no_key, &cache).expect("resolves"),
        "No retrieval key provided."
    );
}

#[test]
fn resolve_returns_no_key_text_for_inputs_without_a_string_key() {
    let cache = cache_map(&[("k", "cached text")]);
    let expected = "No retrieval key provided.";
    let missing_input = object(vec![("id", text("call-1"))]);
    assert_eq!(
        retrieval_content(&missing_input, &cache).expect("resolves"),
        expected
    );
    let null_input = object(vec![("input", Json::Null)]);
    assert_eq!(
        retrieval_content(&null_input, &cache).expect("resolves"),
        expected
    );
    let text_input = object(vec![("input", text("key"))]);
    assert_eq!(
        retrieval_content(&text_input, &cache).expect("resolves"),
        expected
    );
    let empty_input = object(vec![("input", object(Vec::new()))]);
    assert_eq!(
        retrieval_content(&empty_input, &cache).expect("resolves"),
        expected
    );
    let null_key = object(vec![("input", object(vec![("key", Json::Null)]))]);
    assert_eq!(
        retrieval_content(&null_key, &cache).expect("resolves"),
        expected
    );
    let empty_key = object(vec![("input", object(vec![("key", text(""))]))]);
    assert_eq!(
        retrieval_content(&empty_key, &cache).expect("resolves"),
        expected
    );
}

#[test]
fn resolve_rejects_a_call_that_is_not_an_object() {
    let cache = cache_map(&[("k", "cached text")]);
    let error = retrieval_content(&text("call-1"), &cache).expect_err("the call is not an object");
    assert!(matches!(
        error,
        RetrievalLoopError::FieldType {
            field: "tool_calls"
        }
    ));
}

#[test]
fn followup_drops_internal_and_logging_kwargs() {
    let kwargs = index(vec![
        ("litellm_call_id", text("call-1")),
        ("_compression_interception", text("flag")),
        ("_compression_interception_depth", number(1)),
        ("litellm_logging_obj", object(Vec::new())),
        ("x_compression_interception", text("kept")),
    ]);
    let kept = followup_kwargs(&kwargs).expect("the kwargs filter");
    assert_eq!(
        raw_json(&kept),
        index(vec![
            ("litellm_call_id", text("call-1")),
            ("x_compression_interception", text("kept")),
        ])
    );
    assert_eq!(
        kept.keys().map(String::as_str).collect::<Vec<_>>(),
        ["litellm_call_id", "x_compression_interception"]
    );
}

#[test]
fn plan_appends_the_assistant_message_after_the_originals() {
    let interception = interception(true);
    interception
        .cache
        .store("call-1".to_owned(), cache_map(&[("k", "cached text")]));
    let thinking = object(vec![("type", text("thinking")), ("thinking", text("hm"))]);
    let blocks = loop_tools(
        vec![retrieval_call("call-1", Some("k"))],
        vec![thinking.clone()],
    );
    let messages = vec![object(vec![("role", text("user")), ("name", text("kept"))])];
    let params = index(Vec::new());
    let kwargs = index(vec![("litellm_call_id", text("call-1"))]);
    let built = plan(
        &interception,
        &blocks,
        &request(&messages, &params, None, &kwargs),
    );
    let assistant = object(vec![
        ("role", text("assistant")),
        (
            "content",
            list(vec![thinking, retrieval_call("call-1", Some("k"))]),
        ),
    ]);
    let appended = followups(&built);
    assert_eq!(appended.len(), 3);
    assert_eq!(appended[0], messages[0].clone());
    assert_eq!(appended[1], assistant);
}

#[test]
fn plan_builds_the_tool_results_in_call_order() {
    let interception = interception(true);
    interception
        .cache
        .store("call-1".to_owned(), cache_map(&[("k", "cached text")]));
    let blocks = loop_tools(
        vec![
            retrieval_call("call-1", Some("k")),
            retrieval_call("call-2", Some("gone")),
        ],
        Vec::new(),
    );
    let params = index(Vec::new());
    let kwargs = index(vec![("litellm_call_id", text("call-1"))]);
    let built = plan(
        &interception,
        &blocks,
        &request(&[], &params, None, &kwargs),
    );
    let expected = vec![
        tool_result(text("call-1"), "cached text"),
        tool_result(text("call-2"), "[compressed content key 'gone' not found]"),
    ];
    assert_eq!(tool_results(&built), expected);
}

#[test]
fn plan_defaults_a_missing_id_name_and_input() {
    let interception = interception(true);
    let blocks = loop_tools(
        vec![
            object(vec![("id", text("call-1"))]),
            object(vec![("id", text("call-2")), ("name", Json::Null)]),
            object(vec![("input", object(vec![("key", text("k"))]))]),
        ],
        Vec::new(),
    );
    let params = index(Vec::new());
    let kwargs = index(Vec::new());
    let built = plan(
        &interception,
        &blocks,
        &request(&[], &params, None, &kwargs),
    );
    let assistant = object(vec![
        ("role", text("assistant")),
        (
            "content",
            list(vec![
                tool_use(text("call-1"), object(Vec::new())),
                tool_use(text("call-2"), object(Vec::new())),
                tool_use(Json::Null, object(vec![("key", text("k"))])),
            ]),
        ),
    ]);
    assert_eq!(followups(&built)[1], assistant);
    let expected = vec![
        tool_result(text("call-1"), "No retrieval key provided."),
        tool_result(text("call-2"), "No retrieval key provided."),
        tool_result(Json::Null, "[compressed content key 'k' not found]"),
    ];
    assert_eq!(tool_results(&built), expected);
}

#[test]
fn plan_sets_no_tools_and_the_plan_defaults() {
    let interception = interception(true);
    let params = index(Vec::new());
    let kwargs = index(Vec::new());
    let thinking_only = index(vec![("thinking_blocks", list(Vec::new()))]);
    let built = plan(
        &interception,
        &thinking_only,
        &request(&[], &params, None, &kwargs),
    );
    assert!(built.run_agentic_loop);
    assert!(built.response_override.is_none());
    assert!(!built.terminate);
    assert!(built.stop_reason.is_none());
    assert!(tool_results(&built).is_empty());
    let patch = built.request_patch.expect("the patch");
    assert!(patch.tools.is_none());
    assert!(patch.kwargs.is_empty());
    let empty = index(Vec::new());
    let no_calls = plan(&interception, &empty, &request(&[], &params, None, &kwargs));
    assert!(tool_results(&no_calls).is_empty());
}

#[test]
fn plan_patch_kwargs_drop_internal_names() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(Vec::new());
    let kwargs = index(vec![
        ("litellm_call_id", text("call-1")),
        ("_compression_interception", text("flag")),
        ("litellm_logging_obj", object(Vec::new())),
    ]);
    let patch = plan(&interception, &tools, &request(&[], &params, None, &kwargs))
        .request_patch
        .expect("the patch");
    assert_eq!(
        raw_json(&patch.kwargs),
        index(vec![("litellm_call_id", text("call-1"))])
    );
}

#[test]
fn plan_reads_the_logging_call_id_before_the_kwargs_one() {
    let interception = interception(true);
    interception
        .cache
        .store("logged".to_owned(), cache_map(&[("k", "from logging")]));
    let tools = loop_tools(vec![retrieval_call("call-1", Some("k"))], Vec::new());
    let params = index(Vec::new());
    let logged = logging("logged", None);
    let kwargs = index(vec![("litellm_call_id", text("from-kwargs"))]);
    let built = plan(
        &interception,
        &tools,
        &request(&[], &params, Some(&logged), &kwargs),
    );
    assert_eq!(
        raw_json(&built.metadata).get("call_id"),
        Some(&text("logged"))
    );
    assert_eq!(
        tool_results(&built),
        vec![tool_result(text("call-1"), "from logging")]
    );
    let from_kwargs = plan(&interception, &tools, &request(&[], &params, None, &kwargs));
    assert_eq!(
        raw_json(&from_kwargs.metadata).get("call_id"),
        Some(&text("from-kwargs"))
    );
    let missing = vec![tool_result(
        text("call-1"),
        "[compressed content key 'k' not found]",
    )];
    assert_eq!(tool_results(&from_kwargs), missing);
}

#[test]
fn plan_metadata_call_id_is_empty_without_an_id() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(Vec::new());
    let kwargs = index(vec![("max_tokens", number(8))]);
    let built = plan(&interception, &tools, &request(&[], &params, None, &kwargs));
    assert_eq!(
        raw_json(&built.metadata),
        index(vec![
            ("tool_type", text("compression_retrieval")),
            ("call_id", text("")),
        ])
    );
}

#[test]
fn plan_max_tokens_prefers_a_nonzero_optional_param() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(vec![
        ("max_tokens", number(100)),
        ("temperature", number(0)),
    ]);
    let kwargs = index(vec![("max_tokens", number(512))]);
    let patch = plan(&interception, &tools, &request(&[], &params, None, &kwargs))
        .request_patch
        .expect("the patch");
    assert_eq!(patch.max_tokens, Some(100));
    assert_eq!(
        raw_json(&patch.optional_params),
        index(vec![("temperature", number(0))])
    );
}

#[test]
fn plan_max_tokens_falls_through_a_zero_or_null_optional_param() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let kwargs = index(vec![("max_tokens", number(512))]);
    let zero = index(vec![("max_tokens", number(0))]);
    let patch = plan(&interception, &tools, &request(&[], &zero, None, &kwargs))
        .request_patch
        .expect("the patch");
    assert_eq!(patch.max_tokens, Some(512));
    let null = index(vec![("max_tokens", Json::Null)]);
    let zero_kwargs = index(vec![("max_tokens", number(0))]);
    let null_patch = plan(
        &interception,
        &tools,
        &request(&[], &null, None, &zero_kwargs),
    )
    .request_patch
    .expect("the patch");
    assert_eq!(null_patch.max_tokens, Some(0));
    let absent = index(Vec::new());
    let no_kwargs = index(Vec::new());
    let absent_patch = plan(
        &interception,
        &tools,
        &request(&[], &absent, None, &no_kwargs),
    )
    .request_patch
    .expect("the patch");
    assert_eq!(absent_patch.max_tokens, None);
}

#[test]
fn plan_model_comes_from_agentic_loop_params() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(Vec::new());
    let kwargs = index(Vec::new());
    let logged = logging("call-1", Some("claude-original"));
    let patch = plan(
        &interception,
        &tools,
        &request(&[], &params, Some(&logged), &kwargs),
    )
    .request_patch
    .expect("the patch");
    assert_eq!(patch.model.as_deref(), Some("claude-original"));
    let empty = logging("call-1", None);
    let empty_logging_patch = plan(
        &interception,
        &tools,
        &request(&[], &params, Some(&empty), &kwargs),
    )
    .request_patch
    .expect("the patch");
    assert_eq!(empty_logging_patch.model.as_deref(), Some(MODEL));
    let plain_patch = plan(&interception, &tools, &request(&[], &params, None, &kwargs))
        .request_patch
        .expect("the patch");
    assert_eq!(plain_patch.model.as_deref(), Some(MODEL));
}

#[test]
fn plan_model_falls_back_for_null_and_rejects_other_types() {
    let interception = interception(true);
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(Vec::new());
    let kwargs = index(Vec::new());
    let null_model = AgenticLogging {
        call_id: None,
        agentic_loop_params: Some(raw_map(vec![("model", Json::Null)])),
    };
    let patch = plan(
        &interception,
        &tools,
        &request(&[], &params, Some(&null_model), &kwargs),
    )
    .request_patch
    .expect("the patch");
    assert_eq!(patch.model.as_deref(), Some(MODEL));
    let wrong_model = AgenticLogging {
        call_id: None,
        agentic_loop_params: Some(raw_map(vec![("model", number(4))])),
    };
    let error = interception
        .build_agentic_loop_plan(&tools, &request(&[], &params, Some(&wrong_model), &kwargs))
        .expect_err("the plan fails");
    assert!(matches!(
        error,
        RetrievalLoopError::FieldType { field: "model" }
    ));
}

#[test]
fn plan_prunes_expired_entries_before_reading() {
    let (clock, now) = manual_clock(NOW);
    let interception = interception_at(clock);
    interception
        .cache
        .store("call-1".to_owned(), cache_map(&[("k", "cached text")]));
    let tools = loop_tools(vec![retrieval_call("call-1", Some("k"))], Vec::new());
    let params = index(Vec::new());
    let kwargs = index(vec![("litellm_call_id", text("call-1"))]);
    let built_request = request(&[], &params, None, &kwargs);
    *now.lock().expect("the clock") = 1900.0_f64;
    let found = vec![tool_result(text("call-1"), "cached text")];
    assert_eq!(
        tool_results(&plan(&interception, &tools, &built_request)),
        found
    );
    *now.lock().expect("the clock") = 1900.5_f64;
    let missed = vec![tool_result(
        text("call-1"),
        "[compressed content key 'k' not found]",
    )];
    assert_eq!(
        tool_results(&plan(&interception, &tools, &built_request)),
        missed
    );
}

#[test]
fn plan_rejects_a_typed_max_tokens_with_the_field_name() {
    let interception = interception(true);
    let kwargs = index(Vec::new());
    let tools = loop_tools(Vec::new(), Vec::new());
    let params = index(vec![("max_tokens", text("many"))]);
    let error = interception
        .build_agentic_loop_plan(&tools, &request(&[], &params, None, &kwargs))
        .expect_err("the plan fails");
    assert!(matches!(
        error,
        RetrievalLoopError::FieldType {
            field: "max_tokens"
        }
    ));
    let negative = index(vec![("max_tokens", number(-1))]);
    let negative_error = interception
        .build_agentic_loop_plan(&tools, &request(&[], &negative, None, &kwargs))
        .expect_err("the plan fails");
    assert!(matches!(
        negative_error,
        RetrievalLoopError::FieldType {
            field: "max_tokens"
        }
    ));
}

#[test]
fn plan_rejects_typed_tools_with_the_field_name() {
    let interception = interception(true);
    let kwargs = index(Vec::new());
    let params = index(Vec::new());
    let wrong_blocks = index(vec![
        ("tool_calls", list(Vec::new())),
        ("thinking_blocks", text("none")),
    ]);
    let error = interception
        .build_agentic_loop_plan(&wrong_blocks, &request(&[], &params, None, &kwargs))
        .expect_err("the plan fails");
    assert!(matches!(
        error,
        RetrievalLoopError::FieldType {
            field: "thinking_blocks"
        }
    ));
    let wrong_calls = index(vec![("tool_calls", text("none"))]);
    let wrong_calls_error = interception
        .build_agentic_loop_plan(&wrong_calls, &request(&[], &params, None, &kwargs))
        .expect_err("the plan fails");
    assert!(matches!(
        wrong_calls_error,
        RetrievalLoopError::FieldType {
            field: "tool_calls"
        }
    ));
}
