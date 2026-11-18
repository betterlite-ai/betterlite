use betterlite_prompts::arguments::json::Json;

use super::support::{CLAUDE_3_5, CLAUDE_4_5, Call, LLAMA, expected, field, remote, run};

// The body the source builds from mapped options, including the fields it moves out of the way.
pub(super) fn converse_body_shape() {
    let optional = r#"{
        "_output_config_normalized": false,
        "json_mode": true,
        "maxTokens": 256,
        "stopSequences": ["END"],
        "temperature": 0.5,
        "topP": 0.9,
        "topK": 5,
        "top_k": 3,
        "custom_field": {"a": 1},
        "parallel_tool_calls": true,
        "client_metadata": {"team": "x"},
        "guardrailConfig": {"guardrailIdentifier": "gr-1"},
        "requestMetadata": {"client_key": "caller"}
    }"#;
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, optional), &remote(&[]));
    assert_eq!(
        outcome.body(),
        &expected(
            r#"{
                "messages": [{"role": "user", "content": [{"guardContent": {"text": {"text": "hi"}}}]}],
                "inferenceConfig": {
                    "maxTokens": 256, "stopSequences": ["END"], "temperature": 0.5,
                    "topP": 0.9, "topK": 3
                },
                "additionalModelRequestFields": {"custom_field": {"a": 1}, "top_k": 5},
                "guardrailConfig": {"guardrailIdentifier": "gr-1"},
                "requestMetadata": {"client_key": "caller"}
            }"#,
        )
    );
    // Only the internal marker leaves the caller's own options; the copy takes the rest.
    assert!(field(&outcome.optional_params, "_output_config_normalized").is_none());
    assert_eq!(
        field(&outcome.optional_params, "json_mode"),
        Some(&expected("true"))
    );
    assert_eq!(
        field(&outcome.optional_params, "requestMetadata"),
        Some(&expected(r#"{"client_key": "caller"}"#))
    );
    assert_eq!(
        field(&outcome.optional_params, "topK"),
        Some(&expected("5"))
    );

    // inferenceConfig is always there, and null-valued pops leave no field behind.
    let (empty, _warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{
                "requestMetadata": null,
                "outputConfig": null,
                "performanceConfig": null,
                "serviceTier": null
            }"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        empty.body(),
        &expected(
            r#"{
                "messages": [{"role": "user", "content": [{"text": "hi"}]}],
                "inferenceConfig": {}
            }"#,
        )
    );
}

// A tool_config breakpoint, its attribution credit, and the toolChoice rewrite.
pub(super) fn tool_config_injection() {
    let optional = r#"{
        "tools": [{"type": "function", "function": {
            "name": "lookup",
            "description": "finds things",
            "parameters": {"type": "object", "properties": {}}
        }}],
        "tool_choice": {"tool": {"name": "lookup"}},
        "_parallel_tool_use_config": {"tool_choice": {"type": "auto", "disable_parallel_tool_use": true}},
        "cache_control_injection_points": [
            {"location": "tool_config", "control": {"ttl": "1h"}},
            {"location": "tool_config", "control": {"type": "ephemeral"}}
        ]
    }"#;
    let litellm = r#"{"drop_params": true, "metadata": {"team": "core"}, "model_info": {"id": "deployment-1"}}"#;
    let call = Call {
        litellm_params: Some(litellm),
        ..Call::new(CLAUDE_4_5, optional)
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        outcome.body(),
        &expected(
            r#"{
                "messages": [{"role": "user", "content": [{"text": "hi"}]}],
                "inferenceConfig": {},
                "additionalModelRequestFields": {"tool_choice": {"disable_parallel_tool_use": true}},
                "toolConfig": {
                    "tools": [
                        {"toolSpec": {
                            "name": "lookup",
                            "description": "finds things",
                            "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}
                        }},
                        {"cachePoint": {"type": "default", "ttl": "1h"}}
                    ],
                    "toolChoice": {"tool": {"name": "lookup"}}
                }
            }"#,
        )
    );
    assert_eq!(
        outcome.litellm_params,
        Some(expected(
            r#"{
                "drop_params": true,
                "metadata": {"team": "core", "litellm_gateway_injected_cache": "deployment-1"},
                "model_info": {"id": "deployment-1"}
            }"#,
        ))
    );

    // A point outside tool_config places nothing and spends no credit.
    let elsewhere = optional.replace(
        r#"{"location": "tool_config", "control": {"ttl": "1h"}},
            {"location": "tool_config", "control": {"type": "ephemeral"}}"#,
        r#"{"location": "system"}"#,
    );
    let call = Call {
        litellm_params: Some(litellm),
        ..Call::new(CLAUDE_4_5, &elsewhere)
    };
    let (unplaced, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(unplaced.body(), "toolConfig"),
        Some(&expected(
            r#"{"tools": [{"toolSpec": {
                "name": "lookup",
                "description": "finds things",
                "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}
            }}], "toolChoice": {"tool": {"name": "lookup"}}}"#,
        ))
    );
    assert_eq!(
        unplaced.litellm_params,
        Some(expected(
            r#"{"drop_params": true, "metadata": {"team": "core"}, "model_info": {"id": "deployment-1"}}"#,
        ))
    );

    // A model that advertises no prompt caching keeps the tools and spends no credit.
    let call = Call {
        litellm_params: Some(litellm),
        ..Call::new(LLAMA, optional)
    };
    let (uncached, _warnings) = run(&call, &remote(&[]));
    let tools = field(uncached.body(), "toolConfig")
        .and_then(|config| field(config, "tools"))
        .expect("the toolConfig carries tools");
    let Json::List(tools) = tools else {
        panic!("the toolConfig tools are a list");
    };
    assert_eq!(tools.len(), 1);
    assert_eq!(
        uncached.litellm_params,
        Some(expected(
            r#"{"drop_params": true, "metadata": {"team": "core"}, "model_info": {"id": "deployment-1"}}"#,
        ))
    );

    // Without converted tools there is no toolConfig, so tool_choice stays an inference field.
    let (choiceless, _warnings) = run(
        &Call::new(
            CLAUDE_4_5,
            r#"{"tool_choice": {"tool": {"name": "lookup"}}}"#,
        ),
        &remote(&[]),
    );
    assert!(field(choiceless.body(), "toolConfig").is_none());
    assert_eq!(
        field(choiceless.body(), "inferenceConfig"),
        Some(&expected(
            r#"{"tool_choice": {"tool": {"name": "lookup"}}}"#
        ))
    );
}

// The compact-only context management filter runs inside the tools stage and its beta reaches the body.
pub(super) fn context_management_filter() {
    let optional = r#"{
        "custom": 1,
        "context_management": {"edits": [
            {"type": "clear_tool_uses_20250919"},
            {"type": "compact_20260112"}
        ]}
    }"#;
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, optional), &remote(&[]));
    assert_eq!(
        field(outcome.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{
                "custom": 1,
                "context_management": {"edits": [{"type": "compact_20260112"}]},
                "anthropic_beta": ["compact-2026-01-12"]
            }"#,
        ))
    );

    // An edits list without a compact entry drops the whole field and adds no beta.
    let (dropped, _warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{"custom": 1, "context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(dropped.body(), "additionalModelRequestFields"),
        Some(&expected(r#"{"custom": 1}"#))
    );

    // A model outside the Anthropic base keeps the field untouched: the filter is gated on the family.
    let (foreign, _warnings) = run(
        &Call::new(
            LLAMA,
            r#"{"custom": 1, "context_management": {"edits": []}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(foreign.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"custom": 1, "context_management": {"edits": []}}"#,
        ))
    );
}
