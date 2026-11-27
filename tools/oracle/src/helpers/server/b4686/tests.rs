#![cfg(test)]

use betterlite_server::mcp::tool_search::identity::mcp_proxy_tool_id;
use serde_json::Value as JsonValue;
use serde_json::json;

use super::input::Input;
use super::run::run;

fn decode(value: JsonValue) -> Input {
    serde_json::from_value(value).expect("the helper input decodes")
}

fn text_of(value: &JsonValue) -> &str {
    value
        .pointer("/content/0/text")
        .and_then(JsonValue::as_str)
        .expect("a text content block")
}

#[test]
fn agent_search_returns_hits_for_the_clamped_top_k() {
    let input = decode(json!({
        "handler": "agent_search",
        "query": "alpha agents",
        "top_k": 0_i64,
        "agents": [{"id": "a1", "name": "Agent One"}, {"id": "a2", "name": "Agent Two"}],
        "agent_embedding_model": "embed-small",
        "caller": {"user_id": "u1", "team_id": "t1", "end_user_id": "e1"},
    }));
    let value = run(&input).expect("the hits envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/content/0/json/0/agent_id"),
        Some(&json!("a1"))
    );
    assert_eq!(
        value.pointer("/content/0/json/1/skills/0/name"),
        Some(&json!("skill-a2"))
    );
    assert_eq!(
        value.pointer("/observed/agent_searches/0/top_k"),
        Some(&json!(1_i64))
    );
    assert_eq!(
        value.pointer("/observed/agent_searches/0/query"),
        Some(&json!("alpha agents"))
    );
    assert_eq!(
        value.pointer("/observed/agent_searches/0/embedding_model"),
        Some(&json!("embed-small"))
    );
    assert_eq!(
        value.pointer("/observed/feature_checks/0/feature"),
        Some(&json!("agents"))
    );
    assert_eq!(
        value.pointer("/observed/agent_searches/0/caller/end_user_id"),
        Some(&json!("e1"))
    );
}

#[test]
fn agent_search_feature_denial_is_a_403_error() {
    let input = decode(json!({
        "handler": "agent_search",
        "query": "alpha",
        "feature_denied": true,
    }));
    let error = run(&input).expect_err("the feature denial");
    assert_eq!(error.status, Some(403));
}

#[test]
fn agent_search_not_configured_reason_is_error_text() {
    let input = decode(json!({
        "handler": "agent_search",
        "query": "alpha",
        "agents": [{"id": "a1", "name": "Agent One"}],
        "agent_outcome": "not_configured",
        "agent_reason": "agent search needs an embedding model",
    }));
    let value = run(&input).expect("the not-configured envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(true)));
    assert_eq!(text_of(&value), "agent search needs an embedding model");
}

#[test]
fn skill_search_clamps_top_k_to_the_maximum() {
    let input = decode(json!({
        "handler": "skill_search",
        "query": "alpha",
        "top_k": 500_i64,
        "skills": [{"id": "s1", "name": "Skill One", "description": "does alpha"}],
        "caller": {"user_id": "u1"},
    }));
    let value = run(&input).expect("the hits envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/observed/skill_searches/0/top_k"),
        Some(&json!(100_i64))
    );
    assert_eq!(
        value.pointer("/content/0/json/0/display_title"),
        Some(&json!("Skill One"))
    );
}

#[test]
fn invalid_tool_search_settings_become_error_text_without_listing() {
    let input = decode(json!({
        "handler": "mcp_tool_search",
        "query": "alpha",
        "settings": {"top_k": 0_i64},
        "tools": [{"name": "alpha-tool", "server_id": "srv"}],
    }));
    let value = run(&input).expect("the invalid settings envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(true)));
    assert!(text_of(&value).starts_with("litellm_settings.mcp_tool_search is invalid: "));
    assert_eq!(value.pointer("/observed/list_calls/0"), None);
}

#[test]
fn tool_search_needs_a_router_for_a_configured_embedding_model() {
    let input = decode(json!({
        "handler": "mcp_tool_search",
        "query": "alpha",
        "settings": {"embedding_model": "embed-small"},
        "tools": [{"name": "alpha-tool", "server_id": "srv"}],
    }));
    let value = run(&input).expect("the missing router envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(true)));
    assert_eq!(
        text_of(&value),
        "litellm_settings.mcp_tool_search.embedding_model needs a model_list so it can be called"
    );
    assert_eq!(value.pointer("/observed/list_calls/0"), None);
}

#[test]
fn semantic_tool_search_records_one_embedding_batch() {
    let input = decode(json!({
        "handler": "mcp_tool_search",
        "query": "alpha",
        "settings": {"embedding_model": "embed-small", "top_k": 5_i64},
        "has_router": true,
        "tools": [
            {"name": "alpha-tool", "server_id": "srv", "description": "finds alpha"},
            {"name": "beta-tool", "server_id": "srv", "description": "finds beta"},
        ],
    }));
    let value = run(&input).expect("the ranked envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/content/0/json/0/name"),
        Some(&json!("alpha-tool"))
    );
    assert_eq!(
        value.pointer("/content/0/json/0/score"),
        Some(&json!(1.0_f64))
    );
    assert_eq!(
        value.pointer("/observed/embeddings/0/model"),
        Some(&json!("embed-small"))
    );
    assert_eq!(
        value.pointer("/observed/embeddings/0/texts/1"),
        Some(&json!("alpha-tool\nfinds alpha"))
    );
}

#[test]
fn proxy_search_ranks_matching_tools_by_keyword() {
    let input = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "search_tools",
        "arguments": {"query": "alpha"},
        "settings": {"top_k": 5_i64},
        "tools": [
            {"name": "alpha-tool", "server_id": "srv", "description": "finds alpha"},
            {"name": "beta-tool", "server_id": "srv", "description": "finds beta"},
        ],
        "caller": {"user_id": "u1"},
    }));
    let value = run(&input).expect("the proxy search envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/content/0/json/0/name"),
        Some(&json!("alpha-tool"))
    );
    assert_eq!(value.pointer("/content/0/json/1"), None);
    assert_eq!(
        value.pointer("/observed/list_calls/0/proxy_mode"),
        Some(&json!(true))
    );
}

#[test]
fn proxy_schema_returns_the_served_schemas() {
    let listing = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "get_tool_schema",
        "tools": [{
            "name": "alpha-tool",
            "server_id": "srv",
            "description": "finds alpha",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
            "outputSchema": {"type": "object"},
        }],
        "caller": {"user_id": "u1"},
    }));
    let tool_id = mcp_proxy_tool_id(&listing.listed_tools()[0]).expect("the tool identity");
    let request = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "get_tool_schema",
        "arguments": {"tool_id": tool_id},
        "tools": [{
            "name": "alpha-tool",
            "server_id": "srv",
            "description": "finds alpha",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
            "outputSchema": {"type": "object"},
        }],
        "caller": {"user_id": "u1"},
    }));
    let value = run(&request).expect("the schema envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(
        value.pointer("/content/0/json/name"),
        Some(&json!("alpha-tool"))
    );
    assert_eq!(
        value.pointer("/content/0/json/inputSchema/required/0"),
        Some(&json!("q"))
    );
    assert_eq!(
        value.pointer("/content/0/json/outputSchema"),
        Some(&json!({"type": "object"}))
    );
}

#[test]
fn proxy_unknown_tool_name_is_a_400_error() {
    let listing = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "search_everything",
        "tools": [{"name": "alpha-tool", "server_id": "srv"}],
        "caller": {"user_id": "u1"},
    }));
    let tool_id = mcp_proxy_tool_id(&listing.listed_tools()[0]).expect("the tool identity");
    let request = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "search_everything",
        "arguments": {"tool_id": tool_id},
        "tools": [{"name": "alpha-tool", "server_id": "srv"}],
        "caller": {"user_id": "u1"},
    }));
    let error = run(&request).expect_err("the unknown proxy name");
    assert_eq!(error.status, Some(400));
}

#[test]
fn proxy_call_rejects_arguments_that_miss_the_schema() {
    let listing = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "call_tool",
        "tools": [{
            "name": "alpha-tool",
            "server_id": "srv",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
        }],
        "caller": {"user_id": "u1"},
    }));
    let tool_id = mcp_proxy_tool_id(&listing.listed_tools()[0]).expect("the tool identity");
    let request = decode(json!({
        "handler": "mcp_proxy_tool",
        "name": "call_tool",
        "arguments": {"tool_id": tool_id, "arguments": {"q": 1_i64}},
        "tools": [{
            "name": "alpha-tool",
            "server_id": "srv",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
        }],
        "caller": {"user_id": "u1"},
    }));
    let value = run(&request).expect("the validation envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(true)));
    assert!(text_of(&value).starts_with("Invalid arguments: "));
    assert_eq!(value.pointer("/observed/executions/0"), None);
}

fn scoped_call_input() -> Input {
    decode(json!({
        "handler": "mcp_tool_call",
        "tool_name": "alpha-tool",
        "call_arguments": {"q": 1_i64},
        "allowed_servers": ["srv"],
        "mcp_servers": ["srv"],
        "requested_server_id": "srv",
        "mcp_auth_header": "Bearer auth",
        "mcp_server_auth_headers": {"srv": {"authorization": "Bearer server"}},
        "oauth2_headers": {"authorization": "Bearer oauth"},
        "raw_headers": {"x-request-id": "abc"},
        "client_ip": "10.0.0.7",
        "guardrail_context": {"model": "gpt-4o", "metadata": {"trace": "t1"}},
        "caller": {"user_id": "u1"},
    }))
}

fn execution_record(value: &JsonValue) -> &JsonValue {
    value
        .pointer("/observed/executions/0")
        .expect("one recorded execution")
}

#[test]
fn tool_call_records_the_guardrail_context_and_request_scope() {
    let value = run(&scoped_call_input()).expect("the execution envelope");
    assert_eq!(value.pointer("/isError"), Some(&json!(false)));
    assert_eq!(text_of(&value), "executed alpha-tool");
    let execution = execution_record(&value);
    assert_eq!(execution.pointer("/tool_name"), Some(&json!("alpha-tool")));
    assert_eq!(execution.pointer("/arguments/q"), Some(&json!(1_i64)));
    assert_eq!(
        execution.pointer("/guardrail_context/model"),
        Some(&json!("gpt-4o"))
    );
    assert_eq!(
        execution.pointer("/guardrail_context/metadata/trace"),
        Some(&json!("t1"))
    );
    assert_eq!(
        execution.pointer("/requested_server_id"),
        Some(&json!("srv"))
    );
    assert_eq!(
        execution.pointer("/scope/mcp_auth_header"),
        Some(&json!("Bearer auth"))
    );
    assert_eq!(
        execution.pointer("/scope/mcp_server_auth_headers/srv/authorization"),
        Some(&json!("Bearer server"))
    );
    assert_eq!(
        execution.pointer("/scope/raw_headers/x-request-id"),
        Some(&json!("abc"))
    );
    assert_eq!(execution.pointer("/scope/mcp_servers"), None);
}

#[test]
fn scoped_denial_is_a_403_error() {
    let input = decode(json!({
        "handler": "mcp_tool_call",
        "tool_name": "alpha-tool",
        "mcp_servers": ["srv"],
        "denial_detail": "the key may not reach srv",
        "caller": {"user_id": "u1"},
    }));
    let error = run(&input).expect_err("the scoped denial");
    assert_eq!(error.status, Some(403));
}

#[test]
fn missing_allowed_servers_is_a_403_error() {
    let input = decode(json!({
        "handler": "mcp_tool_call",
        "tool_name": "alpha-tool",
        "caller": {"user_id": "u1"},
    }));
    let error = run(&input).expect_err("the empty allowed scope");
    assert_eq!(error.status, Some(403));
}
