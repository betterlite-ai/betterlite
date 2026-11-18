use betterlite_prompts::arguments::json::Json;

use super::support::{CLAUDE_3_5, CLAUDE_4_5, Call, LLAMA, expected, field, remote, run};
use crate::converse_request::error::ConverseRequestError;
use crate::error::BedrockError;
use crate::model_routes::get_bedrock_tool_name;

const CALLER_OPTIONS: &str = r#"{
    "_output_config_normalized": false,
    "json_mode": true,
    "skip_mcp_handler": true,
    "mcp_handler_context": {"session": "s"},
    "_skip_mcp_handler": false,
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
    "performanceConfig": {"latency": "optimized"},
    "serviceTier": {"type": "priority"},
    "requestMetadata": {"client_key": "caller"}
}"#;

const CALLER_BODY: &str = r#"{
    "messages": [{"role": "user", "content": [{"guardContent": {"text": {"text": "hi"}}}]}],
    "inferenceConfig": {
        "maxTokens": 256, "stopSequences": ["END"], "temperature": 0.5,
        "topP": 0.9, "topK": 3
    },
    "additionalModelRequestFields": {"custom_field": {"a": 1}, "top_k": 5},
    "guardrailConfig": {"guardrailIdentifier": "gr-1"},
    "performanceConfig": {"latency": "optimized"},
    "serviceTier": {"type": "priority"},
    "requestMetadata": {"client_key": "caller"}
}"#;

const CLIENT_BETA_HEADERS: &str = r#"{"anthropic-beta": "interleaved-thinking-2025-05-14"}"#;

const NULL_OPTIONS: &str = r#"{
    "requestMetadata": null,
    "outputConfig": null,
    "performanceConfig": null,
    "serviceTier": null
}"#;

const NULL_BODY: &str = r#"{
    "messages": [{"role": "user", "content": [{"text": "hi"}]}],
    "inferenceConfig": {}
}"#;

const TOOL_OPTIONS: &str = r#"{
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

const TOOL_LITELLM: &str =
    r#"{"drop_params": true, "metadata": {"team": "core"}, "model_info": {"id": "deployment-1"}}"#;

const TOOL_LITELLM_CREDITED: &str = r#"{
    "drop_params": true,
    "metadata": {"team": "core", "litellm_gateway_injected_cache": "deployment-1"},
    "model_info": {"id": "deployment-1"}
}"#;

const TOOL_BODY: &str = r#"{
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
}"#;

const TOOL_CONFIG_UNPLACED: &str = r#"{"tools": [{"toolSpec": {
    "name": "lookup",
    "description": "finds things",
    "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}
}}], "toolChoice": {"tool": {"name": "lookup"}}}"#;

const CACHE_POINT_ONLY: &str =
    r#"{"cache_control_injection_points": [{"location": "tool_config"}]}"#;

const RENAMED_TOOLS: &str = r#"{
    "tools": [
        {"type": "function", "function": {
            "name": "_lookup",
            "description": "leading underscore",
            "parameters": {"type": "object", "properties": {}}
        }},
        {"type": "function", "function": {
            "name": "123lookup",
            "description": "leading digits",
            "parameters": {"type": "object", "properties": {}}
        }}
    ]
}"#;

const BOUNDED_TOOL: &str = r#"{
    "tools": [{"type": "function", "function": {
        "name": "bounded",
        "description": "a large bound",
        "parameters": {"type": "object", "properties": {
            "bound": {"type": "number", "maximum": 1e30}
        }}
    }}]
}"#;

const CONTEXT_OPTIONS: &str = r#"{
    "custom": 1,
    "context_management": {"edits": [
        {"type": "clear_tool_uses_20250919"},
        {"type": "compact_20260112"}
    ]}
}"#;

const CONTEXT_COMPACTED: &str = r#"{
    "custom": 1,
    "context_management": {"edits": [{"type": "compact_20260112"}]},
    "anthropic_beta": ["compact-2026-01-12"]
}"#;

const CONTEXT_WITHOUT_COMPACT: &str = r#"{
    "custom": 1,
    "context_management": {"edits": [{"type": "clear_tool_uses_20250919"}]}
}"#;

const CONTEXT_FOREIGN: &str = r#"{"custom": 1, "context_management": {"edits": []}}"#;

// The body the source builds from mapped options, including the fields it moves out of the way.
pub(super) fn converse_body_shape() {
    caller_options_move_and_copy();
    null_options_leave_no_field();
    client_beta_header_reaches_the_request_fields();
    invalid_request_metadata_is_rejected();
    null_metadata_members_are_dropped();
}

fn invalid_request_metadata_is_rejected() {
    let (invalid, _warnings) = run(
        &Call::new(CLAUDE_3_5, r#"{"requestMetadata": {"bad!": "x"}}"#),
        &remote(&[]),
    );
    assert!(matches!(
        invalid.error(),
        ConverseRequestError::Metadata(BedrockError::MetadataKeyCharacters { .. })
    ));
    assert_eq!(invalid.error().status_code(), Some(400));
}

// The exception filter LiteLLM runs first drops the null member before the metadata validation.
fn null_metadata_members_are_dropped() {
    let (compacted, _warnings) = run(
        &Call::new(
            CLAUDE_3_5,
            r#"{"requestMetadata": {"dropped": null, "kept": "x"}}"#,
        ),
        &remote(&[]),
    );
    assert_eq!(
        field(compacted.body(), "requestMetadata"),
        Some(&expected(r#"{"kept": "x"}"#))
    );
}

fn client_beta_header_reaches_the_request_fields() {
    let call = Call {
        headers: Some(CLIENT_BETA_HEADERS),
        ..Call::new(CLAUDE_3_5, "{}")
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(outcome.body(), "additionalModelRequestFields"),
        Some(&expected(
            r#"{"anthropic_beta": ["interleaved-thinking-2025-05-14"]}"#
        ))
    );
}

fn caller_options_move_and_copy() {
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, CALLER_OPTIONS), &remote(&[]));
    assert_eq!(outcome.body(), &expected(CALLER_BODY));
    // The members keep the pinned module's insertion order, which the parsed copy also preserves.
    let Json::Object(fields) = outcome.body() else {
        panic!("the Converse body is an object");
    };
    let order: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        order,
        [
            "messages",
            "inferenceConfig",
            "additionalModelRequestFields",
            "guardrailConfig",
            "performanceConfig",
            "serviceTier",
            "requestMetadata"
        ]
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
}

fn null_options_leave_no_field() {
    let (empty, _warnings) = run(&Call::new(CLAUDE_3_5, NULL_OPTIONS), &remote(&[]));
    assert_eq!(empty.body(), &expected(NULL_BODY));
}

// A tool_config breakpoint, its attribution credit, and the toolChoice rewrite.
pub(super) fn tool_config_injection() {
    tool_config_breakpoint();
    tool_config_point_outside_tool_config();
    uncached_model_places_no_breakpoint();
    tool_choice_without_tools_stays_inference();
    cache_points_without_tools_place_nothing();
    tool_names_round_trip_through_the_response_mapping();
    tool_schema_keeps_a_large_bound();
}

fn tool_config_breakpoint() {
    let call = Call {
        litellm_params: Some(TOOL_LITELLM),
        ..Call::new(CLAUDE_4_5, TOOL_OPTIONS)
    };
    let (outcome, _warnings) = run(&call, &remote(&[]));
    assert_eq!(outcome.body(), &expected(TOOL_BODY));
    assert_eq!(
        outcome.litellm_params,
        Some(expected(TOOL_LITELLM_CREDITED))
    );
}

fn tool_config_point_outside_tool_config() {
    let elsewhere = TOOL_OPTIONS.replace(r#""location": "tool_config""#, r#""location": "system""#);
    let call = Call {
        litellm_params: Some(TOOL_LITELLM),
        ..Call::new(CLAUDE_4_5, &elsewhere)
    };
    let (unplaced, _warnings) = run(&call, &remote(&[]));
    assert_eq!(
        field(unplaced.body(), "toolConfig"),
        Some(&expected(TOOL_CONFIG_UNPLACED))
    );
    assert_eq!(unplaced.litellm_params, Some(expected(TOOL_LITELLM)));
}

fn uncached_model_places_no_breakpoint() {
    let call = Call {
        litellm_params: Some(TOOL_LITELLM),
        ..Call::new(LLAMA, TOOL_OPTIONS)
    };
    let (uncached, _warnings) = run(&call, &remote(&[]));
    let tools = field(uncached.body(), "toolConfig")
        .and_then(|config| field(config, "tools"))
        .expect("the toolConfig carries tools");
    let Json::List(entries) = tools else {
        panic!("the toolConfig tools are a list");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(uncached.litellm_params, Some(expected(TOOL_LITELLM)));
}

fn tool_choice_without_tools_stays_inference() {
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

fn cache_points_without_tools_place_nothing() {
    let call = Call {
        litellm_params: Some(TOOL_LITELLM),
        ..Call::new(CLAUDE_4_5, CACHE_POINT_ONLY)
    };
    let (empty, _warnings) = run(&call, &remote(&[]));
    assert!(field(empty.body(), "toolConfig").is_none());
    assert_eq!(empty.litellm_params, Some(expected(TOOL_LITELLM)));
}

// Bedrock only accepts its own name pattern, and the assembly hands back what the reply restores.
fn tool_names_round_trip_through_the_response_mapping() {
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, RENAMED_TOOLS), &remote(&[]));
    let tool_config =
        field(outcome.body(), "toolConfig").expect("the request carries a toolConfig");
    let Json::List(tools) = field(tool_config, "tools").expect("the toolConfig carries tools")
    else {
        panic!("the toolConfig tools are a list");
    };
    let sent: Vec<&str> = tools
        .iter()
        .map(|tool| {
            let spec = field(tool, "toolSpec").expect("a tool carries its spec");
            let Json::Text(name) = field(spec, "name").expect("a tool spec carries its name")
            else {
                panic!("the tool name is text");
            };
            name.as_str()
        })
        .collect();
    assert_eq!(sent, ["a_lookup", "a123lookup"]);
    let restored: Vec<&str> = sent
        .iter()
        .map(|name| get_bedrock_tool_name(&outcome.tool_name_mappings, name))
        .collect();
    assert_eq!(restored, ["_lookup", "123lookup"]);
}

// A schema bound beyond Decimal's range passes the record roundtrip and stays a JSON number.
fn tool_schema_keeps_a_large_bound() {
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, BOUNDED_TOOL), &remote(&[]));
    let tool = field(outcome.body(), "toolConfig")
        .and_then(|config| field(config, "tools"))
        .and_then(|tools| match tools {
            Json::List(items) => items.first(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
        })
        .expect("the toolConfig carries a tool");
    let maximum = field(tool, "toolSpec")
        .and_then(|spec| field(spec, "inputSchema"))
        .and_then(|schema| field(schema, "json"))
        .and_then(|json| field(json, "properties"))
        .and_then(|properties| field(properties, "bound"))
        .and_then(|bound| field(bound, "maximum"));
    assert_eq!(maximum, Some(&expected("1e30")));
}

// The compact-only context management filter runs inside the tools stage and its beta reaches the body.
pub(super) fn context_management_filter() {
    compact_edit_keeps_beta();
    edits_without_compact_drop();
    foreign_model_keeps_context_management();
}

fn compact_edit_keeps_beta() {
    let (outcome, _warnings) = run(&Call::new(CLAUDE_3_5, CONTEXT_OPTIONS), &remote(&[]));
    assert_eq!(
        field(outcome.body(), "additionalModelRequestFields"),
        Some(&expected(CONTEXT_COMPACTED))
    );
}

fn edits_without_compact_drop() {
    let (dropped, _warnings) = run(
        &Call::new(CLAUDE_3_5, CONTEXT_WITHOUT_COMPACT),
        &remote(&[]),
    );
    assert_eq!(
        field(dropped.body(), "additionalModelRequestFields"),
        Some(&expected(r#"{"custom": 1}"#))
    );
}

fn foreign_model_keeps_context_management() {
    let (foreign, _warnings) = run(&Call::new(LLAMA, CONTEXT_FOREIGN), &remote(&[]));
    assert_eq!(
        field(foreign.body(), "additionalModelRequestFields"),
        Some(&expected(CONTEXT_FOREIGN))
    );
}
