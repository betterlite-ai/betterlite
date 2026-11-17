use crate::converse_params::cache_point::{build_cache_point_block, get_cache_point_block};
use crate::converse_params::inference::{handle_top_k_value, transform_inference_params};
use crate::converse_params::mapping::{map_request_metadata_param, map_service_tier_param};
use crate::converse_params::merge::{
    drop_tool_choice_type, merge_parallel_tool_use_config, parallel_tool_use_config,
};
use crate::model_routes::ParamMap;

use super::scenario::{ANTHROPIC, DROP_CALL, DROP_GLOBAL, KEEP, MODEL_COST, mapped};
use super::support::{Answers, Call, Fixture, json, params, run, value};

pub(super) fn inference_config_options() {
    max_tokens_and_stop();
    response_format_and_tools();
    sampling_gates();
}

fn max_tokens_and_stop() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    let model = "openai.gpt-5";
    for (request, expected) in [
        (
            r#"{"max_tokens": 5, "stream": true, "stop": "END"}"#,
            r#"{"maxTokens": 16, "stream": true, "stopSequences": ["END"]}"#,
        ),
        (r#"{"max_completion_tokens": 100}"#, r#"{"maxTokens": 100}"#),
        (r#"{"max_tokens": true}"#, r#"{"maxTokens": 16}"#),
        (r#"{"max_tokens": 12.5}"#, r#"{"maxTokens": 12.5}"#),
        (r#"{"max_tokens": "big"}"#, r#"{"maxTokens": "big"}"#),
        (
            r#"{"max_tokens": 5, "max_completion_tokens": 50}"#,
            r#"{"maxTokens": 50}"#,
        ),
        (
            r#"{"max_completion_tokens": 50, "max_tokens": 5}"#,
            r#"{"maxTokens": 16}"#,
        ),
        (r#"{"stop": ""}"#, "{}"),
        (r#"{"stop": []}"#, r#"{"stopSequences": []}"#),
        (r#"{"stop": null}"#, r#"{"stopSequences": null}"#),
        (r#"{"stop": 42}"#, r#"{"stopSequences": 42}"#),
        (r#"{"logprobs": true}"#, "{}"),
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, model, request).mapped(),
            params(expected),
            "{request}"
        );
    }
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"max_tokens": 5}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 5}"#)
    );
}

fn response_format_and_tools() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    let tools = r#"{"tools": [{"type": "function", "function": {"name": "lookup", "parameters": {"type": "object"}}}]}"#;
    for (request, expected) in [
        (
            r#"{"response_format": {"type": "json_object"}}"#,
            r#"{"json_mode": true}"#,
        ),
        (r#"{"response_format": "json"}"#, "{}"),
        (tools, tools),
        (r#"{"tools": {"type": "function"}}"#, "{}"),
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, ANTHROPIC, request).mapped(),
            params(expected),
            "{request}"
        );
    }
}

fn sampling_gates() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"temperature": 0.5, "top_p": 0.9}"#
        )
        .mapped(),
        params(r#"{"temperature": 0.5, "topP": 0.9}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "amazon.nova-pro-v1:0",
            r#"{"temperature": 0.5}"#
        )
        .mapped(),
        params(r#"{"temperature": 0.5}"#)
    );
    let rejected = mapped(
        &fixture,
        &mut session,
        &answers,
        "anthropic.claude-opus-4-1-v1:0",
        r#"{"temperature": 0.5}"#,
    );
    let error = rejected.error();
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "anthropic.claude-opus-4-1-v1:0 does not support temperature=0.5. Only temperature=1 is supported. To drop unsupported params, set `litellm.drop_params = True`."
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "anthropic.claude-opus-4-1-v1:0",
            r#"{"temperature": 1}"#
        )
        .mapped(),
        params(r#"{"temperature": 1}"#)
    );
    for drop in [DROP_CALL, DROP_GLOBAL] {
        assert_eq!(
            run(
                &fixture,
                &mut session,
                &answers,
                Call {
                    model: "anthropic.claude-opus-4-1-v1:0",
                    non_default: params(r#"{"temperature": 0.5}"#),
                    optional: ParamMap::new(),
                    drop,
                },
            )
            .mapped(),
            params("{}")
        );
    }
    let unsupported = mapped(
        &fixture,
        &mut session,
        &answers,
        "cohere.command-r-v1:0",
        r#"{"top_p": 0.5}"#,
    );
    let error = unsupported.error();
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(
        error.to_string(),
        "cohere.command-r-v1:0 does not support top_p=0.5. To drop unsupported params, set `litellm.drop_params = True`."
    );
    assert_eq!(
        run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: "cohere.command-r-v1:0",
                non_default: params(r#"{"temperature": 0.5}"#),
                optional: ParamMap::new(),
                drop: DROP_CALL,
            },
        )
        .mapped(),
        params("{}")
    );
}

pub(super) fn additional_model_fields() {
    parallel_config();
    thinking_branches();
    effort_and_output_config();
    web_search_grounding();
}

fn parallel_config() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    for (request, disable) in [
        (r#"{"parallel_tool_calls": true}"#, false),
        (r#"{"parallel_tool_calls": false}"#, true),
        (r#"{"parallel_tool_calls": 0}"#, true),
        (r#"{"parallel_tool_calls": ""}"#, true),
        (r#"{"parallel_tool_calls": []}"#, true),
        (r#"{"parallel_tool_calls": {}}"#, true),
        (r#"{"parallel_tool_calls": "x"}"#, false),
        (r#"{"parallel_tool_calls": null}"#, true),
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, ANTHROPIC, request).mapped(),
            params(&format!(
                r#"{{"_parallel_tool_use_config": {{"tool_choice": {{"type": "auto", "disable_parallel_tool_use": {disable}}}}}}}"#
            )),
            "{request}"
        );
    }
}

fn thinking_branches() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    let dropped = mapped(
        &fixture,
        &mut session,
        &answers,
        "deepseek.v3-v1:0",
        r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#,
    );
    assert_eq!(
        dropped.debug(),
        [
            "Dropping unsupported `thinking` param for Bedrock model=deepseek.v3-v1:0; it reasons natively."
        ]
    );
    assert_eq!(dropped.mapped(), params("{}"));
    assert_eq!(
        run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: "deepseek.v3-v1:0",
                non_default: params(r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#),
                optional: params(r#"{"thinking": {"type": "enabled"}}"#),
                drop: KEEP,
            },
        )
        .mapped(),
        params(r#"{"thinking": {"type": "enabled"}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "openai.gpt-5",
            r#"{"thinking": {"type": "enabled", "budget_tokens": 2048}}"#
        )
        .mapped(),
        params("{}")
    );
    let r1 = mapped(
        &fixture,
        &mut session,
        &answers,
        "deepseek.r1-v1:0",
        r#"{"reasoning_effort": "high"}"#,
    );
    assert_eq!(
        r1.debug(),
        [
            "Dropping unsupported `reasoning_effort` param for Bedrock model=deepseek.r1-v1:0; it always reasons and rejects it."
        ]
    );
    assert_eq!(r1.mapped(), params("{}"));
    let non_string = mapped(
        &fixture,
        &mut session,
        &answers,
        "deepseek.r1-v1:0",
        r#"{"reasoning_effort": 5}"#,
    );
    assert!(non_string.debug().is_empty());
    assert_eq!(non_string.mapped(), params("{}"));
}

fn effort_and_output_config() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[(
        ANTHROPIC,
        r#"{"bedrock_output_config_effort_ceiling": "medium"}"#,
    )]);
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"reasoning_effort": "high", "max_tokens": 9000}"#
        )
        .mapped(),
        params(r#"{"maxTokens": 9000, "thinking": {"type": "enabled", "budget_tokens": 4096}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"output_config": {"effort": "high"}}"#
        )
        .mapped(),
        params(r#"{"output_config": {"effort": "medium"}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"output_config": {"effort": "low", "extra": 1}}"#
        )
        .mapped(),
        params(r#"{"output_config": {"effort": "low", "extra": 1}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"output_config": "high"}"#
        )
        .mapped(),
        params("{}")
    );
}

fn web_search_grounding() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "amazon.nova-pro-v1:0",
            r#"{"web_search_options": {}}"#
        )
        .mapped(),
        params(r#"{"tools": [{"systemTool": {"name": "nova_grounding"}}]}"#)
    );
    assert_eq!(
        run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: "amazon.nova-pro-v1:0",
                non_default: params(r#"{"web_search_options": {}}"#),
                optional: params(r#"{"tools": [{"type": "function"}]}"#),
                drop: KEEP,
            },
        )
        .mapped(),
        params(r#"{"tools": [{"type": "function"}, {"systemTool": {"name": "nova_grounding"}}]}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"web_search_options": {}}"#
        )
        .mapped(),
        params("{}")
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            "amazon.nova-pro-v1:0",
            r#"{"web_search_options": []}"#
        )
        .mapped(),
        params("{}")
    );
}

pub(super) fn service_tier() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    for (request, tier) in [
        (r#"{"service_tier": "auto"}"#, "default"),
        (r#"{"service_tier": "default"}"#, "default"),
        (r#"{"service_tier": "flex"}"#, "flex"),
        (r#"{"service_tier": "priority"}"#, "priority"),
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, ANTHROPIC, request).mapped(),
            params(&format!(r#"{{"serviceTier": {{"type": "{tier}"}}}}"#)),
            "{request}"
        );
    }
    for request in [
        r#"{"service_tier": "standard"}"#,
        r#"{"service_tier": ""}"#,
        r#"{"service_tier": "AUTO"}"#,
        r#"{"service_tier": 5}"#,
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, ANTHROPIC, request).mapped(),
            params("{}"),
            "{request}"
        );
    }
    assert_eq!(
        run(
            &fixture,
            &mut session,
            &answers,
            Call {
                model: ANTHROPIC,
                non_default: params(r#"{"service_tier": "standard"}"#),
                optional: params(r#"{"serviceTier": {"type": "flex"}}"#),
                drop: KEEP,
            },
        )
        .mapped(),
        params(r#"{"serviceTier": {"type": "flex"}}"#)
    );
    let mut optional = ParamMap::new();
    map_service_tier_param("flex", &mut optional);
    assert_eq!(optional, params(r#"{"serviceTier": {"type": "flex"}}"#));
}

pub(super) fn request_metadata() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"requestMetadata": {"trace": "abc-123"}}"#
        )
        .mapped(),
        params(r#"{"requestMetadata": {"trace": "abc-123"}}"#)
    );
    let boundary = "x".repeat(256);
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            &format!(r#"{{"requestMetadata": {{"{boundary}": "{boundary}"}}}}"#)
        )
        .mapped(),
        params(&format!(
            r#"{{"requestMetadata": {{"{boundary}": "{boundary}"}}}}"#
        ))
    );
    let too_many = (0..17)
        .map(|index| format!(r#""k{index}": "v""#))
        .collect::<Vec<_>>()
        .join(", ");
    for request in [
        format!(r#"{{"requestMetadata": {{{too_many}}}}}"#),
        r#"{"requestMetadata": {"trace": 1}}"#.to_owned(),
        r#"{"requestMetadata": {"": "v"}}"#.to_owned(),
        format!(r#"{{"requestMetadata": {{"{boundary}x": "v"}}}}"#),
        format!(r#"{{"requestMetadata": {{"trace": "{boundary}x"}}}}"#),
        r#"{"requestMetadata": {"bad!key": "v"}}"#.to_owned(),
        r#"{"requestMetadata": {"trace": "bad!value"}}"#.to_owned(),
    ] {
        let error = mapped(&fixture, &mut session, &answers, ANTHROPIC, &request).error();
        assert_eq!(error.status_code(), Some(400), "{request}");
    }
    for request in [
        r#"{"requestMetadata": null}"#,
        r#"{"requestMetadata": ["trace"]}"#,
        r#"{"requestMetadata": "trace"}"#,
    ] {
        assert_eq!(
            mapped(&fixture, &mut session, &answers, ANTHROPIC, request).mapped(),
            params("{}"),
            "{request}"
        );
    }
    let mut optional = ParamMap::new();
    map_request_metadata_param(&value(r#"{"trace": "abc"}"#), &mut optional)
        .expect("metadata maps");
    assert_eq!(optional, params(r#"{"requestMetadata": {"trace": "abc"}}"#));
}

pub(super) fn cache_point_ttl() {
    let fixture = Fixture::new(MODEL_COST);
    let cost = &fixture.model_cost;
    let claude_45 = Some("anthropic.claude-4-5-bedrock");
    let ttl_block = params(r#"{"content": "x", "cache_control": {"ttl": "1h"}}"#);
    assert_eq!(
        get_cache_point_block(&ttl_block, claude_45, cost).expect("the block maps"),
        Some(json(r#"{"cachePoint": {"type": "default", "ttl": "1h"}}"#))
    );
    assert_eq!(
        get_cache_point_block(&ttl_block, Some("anthropic.claude-3-5-sonnet"), cost)
            .expect("the block maps"),
        Some(json(r#"{"cachePoint": {"type": "default"}}"#))
    );
    for request in [
        r#"{"content": "x"}"#,
        r#"{"content": "x", "cache_control": null}"#,
    ] {
        assert_eq!(
            get_cache_point_block(&params(request), claude_45, cost).expect("the block maps"),
            None,
            "{request}"
        );
    }
    for model in [
        "openai.gpt-5",
        "us.openai.gpt-5",
        "anthropic.claude-no-cache",
    ] {
        assert_eq!(
            get_cache_point_block(&ttl_block, Some(model), cost).expect("the block maps"),
            None,
            "{model}"
        );
    }
    assert_eq!(
        get_cache_point_block(&ttl_block, None, cost).expect("the block maps"),
        Some(json(r#"{"cachePoint": {"type": "default"}}"#))
    );
    for ttl in [r#""5m""#, r#""2h""#, "5", "null", r#"["5m"]"#] {
        let block = params(&format!(r#"{{"cache_control": {{"ttl": {ttl}}}}}"#));
        let expected = if ttl == r#""5m""# {
            json(r#"{"cachePoint": {"type": "default", "ttl": "5m"}}"#)
        } else {
            json(r#"{"cachePoint": {"type": "default"}}"#)
        };
        assert_eq!(
            get_cache_point_block(&block, claude_45, cost).expect("the block maps"),
            Some(expected),
            "{ttl}"
        );
    }
    for control in ["false", "0", r#""ephemeral""#, "[]", "{}"] {
        let block = params(&format!(r#"{{"cache_control": {control}}}"#));
        assert_eq!(
            get_cache_point_block(&block, claude_45, cost).expect("the block maps"),
            Some(json(r#"{"cachePoint": {"type": "default"}}"#)),
            "{control}"
        );
    }
    let system = params(
        r#"{"role": "system", "content": "x", "cache_control": {"type": "ephemeral", "ttl": "1h"}}"#,
    );
    let content = params(
        r#"{"type": "image", "source": {"bytes": "AA=="}, "cache_control": {"type": "ephemeral", "ttl": "1h"}}"#,
    );
    let expected = json(r#"{"cachePoint": {"type": "default", "ttl": "1h"}}"#);
    assert_eq!(
        get_cache_point_block(&system, claude_45, cost).expect("the block maps"),
        Some(expected.clone())
    );
    assert_eq!(
        get_cache_point_block(&content, claude_45, cost).expect("the block maps"),
        Some(expected)
    );
    assert_eq!(
        build_cache_point_block(None, claude_45, cost).expect("the block builds"),
        json(r#"{"type": "default"}"#)
    );
}

pub(super) fn top_k() {
    inference_transform();
    top_k_families();
}

fn inference_transform() {
    for (request, expected) in [
        (
            r#"{"top_k": 10, "temperature": 0.2}"#,
            r#"{"topK": 10, "temperature": 0.2}"#,
        ),
        (r#"{"top_k": 10, "topK": 3}"#, r#"{"topK": 10}"#),
        (r#"{"maxTokens": 100}"#, r#"{"maxTokens": 100}"#),
        ("{}", "{}"),
    ] {
        assert_eq!(
            transform_inference_params(params(request)),
            params(expected),
            "{request}"
        );
    }
}

fn top_k_families() {
    let fixture = Fixture::new(MODEL_COST);
    let catalogs = fixture.catalogs();
    let mut inference = params(r#"{"topK": 10, "maxTokens": 100}"#);
    let mapped =
        handle_top_k_value(ANTHROPIC, &mut inference, KEEP, &catalogs).expect("top_k maps");
    assert_eq!(mapped, params(r#"{"top_k": 10}"#));
    assert_eq!(inference, params(r#"{"maxTokens": 100}"#));
    let mut inference = params(r#"{"top_k": 5}"#);
    let mapped = handle_top_k_value("amazon.nova-pro-v1:0", &mut inference, KEEP, &catalogs)
        .expect("top_k maps");
    assert_eq!(mapped, params(r#"{"inferenceConfig": {"topK": 5}}"#));
    assert_eq!(inference, params("{}"));
    let mut inference = params(r#"{"topK": 5, "top_k": 7}"#);
    let mapped = handle_top_k_value("amazon.nova-pro-v1:0", &mut inference, KEEP, &catalogs)
        .expect("top_k maps");
    assert_eq!(mapped, params(r#"{"inferenceConfig": {"topK": 5}}"#));
    assert_eq!(inference, params(r#"{"top_k": 7}"#));
    let mut inference = params(r#"{"topK": null, "top_k": 7}"#);
    let mapped = handle_top_k_value("amazon.nova-pro-v1:0", &mut inference, KEEP, &catalogs)
        .expect("top_k maps");
    assert_eq!(mapped, params("{}"));
    assert_eq!(inference, params(r#"{"top_k": 7}"#));
    let mut inference = params(r#"{"topK": false}"#);
    let mapped = handle_top_k_value("amazon.nova-pro-v1:0", &mut inference, KEEP, &catalogs)
        .expect("top_k maps");
    assert_eq!(mapped, params(r#"{"inferenceConfig": {"topK": false}}"#));
    let mut inference = params(r#"{"topK": 5}"#);
    let mapped = handle_top_k_value("cohere.command-r-v1:0", &mut inference, KEEP, &catalogs)
        .expect("top_k maps");
    assert_eq!(mapped, params("{}"));
    assert_eq!(inference, params("{}"));
    let mut inference = params(r#"{"topK": 5}"#);
    let error = handle_top_k_value(
        "anthropic.claude-opus-4-1-v1:0",
        &mut inference,
        KEEP,
        &catalogs,
    )
    .expect_err("the model rejects top_k");
    assert_eq!(error.status_code(), Some(400));
    assert_eq!(inference, params("{}"));
    let mut inference = params(r#"{"topK": 5}"#);
    let mapped = handle_top_k_value(
        "anthropic.claude-opus-4-1-v1:0",
        &mut inference,
        DROP_CALL,
        &catalogs,
    )
    .expect("top_k drops");
    assert_eq!(mapped, params("{}"));
    assert_eq!(inference, params("{}"));
}

pub(super) fn parallel_tool_config() {
    let generated = parallel_tool_use_config(&value("true"));
    assert_eq!(
        generated,
        params(r#"{"tool_choice": {"type": "auto", "disable_parallel_tool_use": false}}"#)
    );
    let additional = params(
        r#"{"tool_choice": {"type": "tool", "name": "lookup"}, "unrelated": {"keep": true}}"#,
    );
    let merged = merge_parallel_tool_use_config(&additional, &generated);
    assert_eq!(
        merged,
        params(
            r#"{"tool_choice": {"type": "tool", "name": "lookup", "disable_parallel_tool_use": false}, "unrelated": {"keep": true}}"#
        )
    );
    let replaced = merge_parallel_tool_use_config(
        &params(r#"{"tool_choice": {"type": "tool"}, "unrelated": 1}"#),
        &params(r#"{"tool_choice": "auto"}"#),
    );
    assert_eq!(
        replaced,
        params(r#"{"tool_choice": "auto", "unrelated": 1}"#)
    );
    let inserted = merge_parallel_tool_use_config(
        &params(r#"{"unrelated": 1}"#),
        &params(r#"{"tool_choice": {"type": "auto"}}"#),
    );
    assert_eq!(
        inserted,
        params(r#"{"unrelated": 1, "tool_choice": {"type": "auto"}}"#)
    );
    let mut filled = params(r#"{"tool_choice": {"type": "auto"}}"#);
    drop_tool_choice_type(&mut filled);
    assert_eq!(filled, params(r#"{"tool_choice": {}}"#));
    let mut with_name = params(r#"{"tool_choice": {"type": "tool", "name": "lookup"}, "x": 1}"#);
    drop_tool_choice_type(&mut with_name);
    assert_eq!(
        with_name,
        params(r#"{"tool_choice": {"name": "lookup"}, "x": 1}"#)
    );
    let mut absent = params(r#"{"x": 1}"#);
    drop_tool_choice_type(&mut absent);
    assert_eq!(absent, params(r#"{"x": 1}"#));
    let mut textual = params(r#"{"tool_choice": "required"}"#);
    drop_tool_choice_type(&mut textual);
    assert_eq!(textual, params(r#"{"tool_choice": "required"}"#));
}

pub(super) fn context_management() {
    let fixture = Fixture::new(MODEL_COST);
    let mut session = fixture.session();
    let answers = Answers::new(&[]);
    for (request, expected) in [
        (
            r#"[{"type": "compaction", "compact_threshold": 100000, "keep": true}]"#,
            r#"{"context_management": {"edits": [{"type": "compact_20260112", "trigger": {"type": "input_tokens", "value": 100000}, "keep": true}]}}"#,
        ),
        (
            r#"[{"type": "compaction", "compact_threshold": 100000.9}]"#,
            r#"{"context_management": {"edits": [{"type": "compact_20260112", "trigger": {"type": "input_tokens", "value": 100000}}]}}"#,
        ),
        (
            r#"[{"type": "compaction", "compact_threshold": "many"}]"#,
            r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#,
        ),
        (
            r#"{"edits": [{"type": "compact_20260112"}]}"#,
            r#"{"context_management": {"edits": [{"type": "compact_20260112"}]}}"#,
        ),
        ("{}", r#"{"context_management": {}}"#),
        (r#"{"other": 1}"#, r#"{"context_management": {"other": 1}}"#),
    ] {
        assert_eq!(
            mapped(
                &fixture,
                &mut session,
                &answers,
                ANTHROPIC,
                &format!(r#"{{"context_management": {request}}}"#)
            )
            .mapped(),
            params(expected),
            "{request}"
        );
    }
    for request in ["\"edits\"", "5", "true", "null"] {
        assert_eq!(
            mapped(
                &fixture,
                &mut session,
                &answers,
                ANTHROPIC,
                &format!(r#"{{"context_management": {request}}}"#)
            )
            .mapped(),
            params("{}"),
            "{request}"
        );
    }
    let kept = run(
        &fixture,
        &mut session,
        &answers,
        Call {
            model: ANTHROPIC,
            non_default: params(r#"{"context_management": [{"type": "clear"}]}"#),
            optional: params(r#"{"context_management": {"edits": [{"type": "old"}]}}"#),
            drop: KEEP,
        },
    );
    assert_eq!(
        kept.mapped(),
        params(r#"{"context_management": {"edits": [{"type": "old"}]}}"#)
    );
    assert_eq!(
        mapped(
            &fixture,
            &mut session,
            &answers,
            ANTHROPIC,
            r#"{"context_management": [{"type": "compaction"}], "max_tokens": 9000}"#
        )
        .mapped(),
        params(
            r#"{"maxTokens": 9000, "context_management": {"edits": [{"type": "compact_20260112"}]}}"#
        )
    );
}
